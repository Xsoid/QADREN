//! Split of one complete `mysqldump` text into one file for each table
//! (D-027 section 7b.11.1). The split keeps the load order of the dump:
//!
//! 1. `schema.sql`: the dump header, the structure of all tables, and the
//!    temporary structure of views.
//! 2. `tables/<file>.sql`: the data of one table. Each file has the session
//!    settings of the dump header and the matching restore lines, so that
//!    one table can be loaded alone.
//! 3. `objects.sql`: triggers, the final view structure, routines, events,
//!    and the dump footer. They come after all data, so that a restore does
//!    not fire a trigger for each row.
//!
//! The split reads the text line by line and keeps only one line in memory.

use serde::Serialize;
use std::collections::BTreeSet;
use std::fs::File;
use std::io::{self, BufRead, BufWriter, Write};
use std::path::Path;

pub const SCHEMA_FILE: &str = "schema.sql";
pub const OBJECTS_FILE: &str = "objects.sql";
pub const MANIFEST_FILE: &str = "manifest.json";

/// The restore lines for the session settings of a dump header: for each
/// header line `/*!NNNNN SET @OLD_X=@@X ... */;`, the line
/// `/*!NNNNN SET X=@OLD_X */;`, in reverse order. A table file ends with
/// them, as the dump ends with its footer.
fn restore_lines(header: &[u8]) -> Vec<u8> {
    let mut lines = Vec::new();
    for line in String::from_utf8_lossy(header).lines().rev() {
        let Some(at) = line.find(" SET @OLD_") else {
            continue;
        };
        let name: String = line[at + " SET @OLD_".len()..]
            .chars()
            .take_while(|c| c.is_ascii_uppercase() || *c == '_')
            .collect();
        if !name.is_empty() {
            lines.extend_from_slice(
                format!("{} SET {name}=@OLD_{name} */;\n", &line[..at]).as_bytes(),
            );
        }
    }
    lines
}

/// One file of a split dump, in load order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SplitFile {
    /// The path relative to `databases/<database>/`.
    pub path: String,
    /// The table of a data file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub table: Option<String>,
    pub bytes: u64,
    pub md5: String,
}

struct Output {
    writer: BufWriter<File>,
    context: md5::Context,
    bytes: u64,
}

impl Output {
    fn create(path: &Path) -> io::Result<Self> {
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        Ok(Self {
            writer: BufWriter::new(file),
            context: md5::Context::new(),
            bytes: 0,
        })
    }

    fn write(&mut self, data: &[u8]) -> io::Result<()> {
        self.writer.write_all(data)?;
        self.context.consume(data);
        self.bytes += data.len() as u64;
        Ok(())
    }

    fn finish(self, path: String, table: Option<String>) -> io::Result<SplitFile> {
        let file = self
            .writer
            .into_inner()
            .map_err(|error| error.into_error())?;
        file.sync_all()?;
        Ok(SplitFile {
            path,
            table,
            bytes: self.bytes,
            md5: format!("{:x}", self.context.compute()),
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    Schema,
    Objects,
}

/// The table name of a `mysqldump` comment line such as
/// "-- Dumping data for table `name`".
fn quoted_name(line: &str, prefix: &str) -> Option<String> {
    let rest = line.strip_prefix(prefix)?.trim_end();
    let name = rest.strip_prefix('`')?.strip_suffix('`')?;
    Some(name.replace("``", "`")).filter(|name| !name.is_empty())
}

/// The file name of a table. A plain name is kept. Any other name, and a
/// name that differs only in letter case from an earlier one (a macOS
/// volume does not separate them), gets a name from its MD5.
fn table_file(table: &str, used: &mut BTreeSet<String>) -> String {
    let plain = table.len() <= 100
        && !table.starts_with(['-', '.'])
        && table
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'$'));
    let mut name = if plain {
        table.to_owned()
    } else {
        format!("t-{:x}", md5::compute(table.as_bytes()))
    };
    if !used.insert(name.to_ascii_lowercase()) {
        name = format!("t-{:x}", md5::compute(table.as_bytes()));
        if !used.insert(name.clone()) {
            name = format!("t-{:x}", md5::compute(format!("{table}\0{}", used.len())));
            used.insert(name.clone());
        }
    }
    format!("tables/{name}.sql")
}

/// Split one complete dump into `dir`, which must be empty or absent.
/// Returns the files in load order: `schema.sql`, the table files in dump
/// order, then `objects.sql`. A table with two data sections is an error.
pub fn split_dump(mut input: impl BufRead, dir: &Path) -> io::Result<Vec<SplitFile>> {
    std::fs::create_dir_all(dir.join("tables"))?;
    let mut schema = Output::create(&dir.join(SCHEMA_FILE))?;
    let mut objects = Output::create(&dir.join(OBJECTS_FILE))?;
    let mut tables: Vec<SplitFile> = Vec::new();
    let mut seen = BTreeSet::new();
    let mut used = BTreeSet::new();
    // The session settings of the dump header, for each table file.
    let mut header: Vec<u8> = Vec::new();
    let mut header_open = true;
    let mut footer: Vec<u8> = Vec::new();
    let mut section = Section::Schema;
    let mut data: Option<(Output, String, String)> = None;
    let mut line = Vec::new();
    loop {
        line.clear();
        if input.read_until(b'\n', &mut line)? == 0 {
            break;
        }
        // Markers are ASCII comment lines; data lines are not changed.
        let text = if line.starts_with(b"-- ") {
            std::str::from_utf8(&line).ok()
        } else {
            None
        };
        let starts_data = text.and_then(|text| quoted_name(text, "-- Dumping data for table "));
        let to_schema = text.is_some_and(|text| {
            text.starts_with("-- Table structure for table ")
                || text.starts_with("-- Temporary view structure for view ")
                || text.starts_with("-- Temporary table structure for view ")
        });
        let to_objects = text.is_some_and(|text| {
            text.starts_with("-- Final view structure for view ")
                || text.starts_with("-- Dumping routines for database ")
                || text.starts_with("-- Dumping events for database ")
        });
        if starts_data.is_some() || to_schema || to_objects {
            if header_open {
                header_open = false;
                footer = restore_lines(&header);
            }
            // A dump without `UNLOCK TABLES` ends a data section at the
            // next marker.
            if let Some((output, path, table)) = data.take() {
                let mut output = output;
                output.write(&footer)?;
                tables.push(output.finish(path, Some(table))?);
            }
            if to_schema {
                section = Section::Schema;
            }
            if to_objects {
                section = Section::Objects;
            }
        }
        if let Some(table) = starts_data {
            if !seen.insert(table.clone()) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "a table has two data sections",
                ));
            }
            let path = table_file(&table, &mut used);
            let mut output = Output::create(&dir.join(&path))?;
            output.write(&header)?;
            data = Some((output, path, table));
        }
        match &mut data {
            Some((output, _, _)) => {
                output.write(&line)?;
                if line.trim_ascii_end() == b"UNLOCK TABLES;" {
                    if let Some((mut output, path, table)) = data.take() {
                        output.write(&footer)?;
                        tables.push(output.finish(path, Some(table))?);
                    }
                    // Triggers of the table follow its data.
                    section = Section::Objects;
                }
            }
            None => {
                if header_open && line.starts_with(b"/*!") {
                    header.extend_from_slice(&line);
                }
                match section {
                    Section::Schema => schema.write(&line)?,
                    Section::Objects => objects.write(&line)?,
                }
            }
        }
    }
    if let Some((mut output, path, table)) = data.take() {
        output.write(&footer)?;
        tables.push(output.finish(path, Some(table))?);
    }
    let mut files = vec![schema.finish(SCHEMA_FILE.to_owned(), None)?];
    files.extend(tables);
    files.push(objects.finish(OBJECTS_FILE.to_owned(), None)?);
    Ok(files)
}

/// The `manifest.json` of a split database: the load order, the table of
/// each data file, and the size and MD5 of each file. It has no time, so
/// the same files give the same bytes.
pub fn split_manifest(database: &str, files: &[SplitFile]) -> Vec<u8> {
    let manifest = serde_json::json!({
        "format": "qadren-mysqldump-split-1",
        "database": database,
        "restore": "Load the files in load_order into the database in one mysql session, \
                    for example: cat <files in load_order> | mysql <database>. \
                    One table file can also be loaded alone after schema.sql.",
        "load_order": files,
    });
    let mut text = serde_json::to_vec_pretty(&manifest).unwrap_or_default();
    text.push(b'\n');
    text
}
