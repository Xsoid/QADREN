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

#[cfg(test)]
mod tests {
    use super::*;

    /// A small dump in the form of `mysqldump` 8, with a trigger, a view,
    /// a routine, and two tables whose names differ only in letter case.
    const DUMP: &str = "-- MySQL dump 10.13
--
/*!40101 SET @OLD_CHARACTER_SET_CLIENT=@@CHARACTER_SET_CLIENT */;
/*!50503 SET NAMES utf8mb4 */;
/*!40103 SET @OLD_TIME_ZONE=@@TIME_ZONE */;
/*!40103 SET TIME_ZONE='+00:00' */;

--
-- Table structure for table `orders`
--

CREATE TABLE `orders` (`id` int);

--
-- Dumping data for table `orders`
--

LOCK TABLES `orders` WRITE;
INSERT INTO `orders` VALUES (1),(2);
UNLOCK TABLES;
DELIMITER ;;
/*!50003 CREATE*/ /*!50003 TRIGGER `orders_ai` AFTER INSERT ON `orders` FOR EACH ROW SET @x = 1 */;;
DELIMITER ;

--
-- Table structure for table `Orders`
--

CREATE TABLE `Orders` (`id` int);

--
-- Temporary view structure for view `v`
--

CREATE VIEW `v` AS SELECT 1 AS `id`;

--
-- Table structure for table `we``ird/name`
--

CREATE TABLE `we``ird/name` (`id` int);

--
-- Dumping data for table `Orders`
--

LOCK TABLES `Orders` WRITE;
INSERT INTO `Orders` VALUES (3);
UNLOCK TABLES;

--
-- Dumping data for table `we``ird/name`
--

LOCK TABLES `we``ird/name` WRITE;
INSERT INTO `we``ird/name` VALUES (4);
UNLOCK TABLES;

--
-- Final view structure for view `v`
--

/*!50001 CREATE VIEW `v` AS select 1 */;

--
-- Dumping routines for database 'example'
--
DELIMITER ;;
CREATE PROCEDURE `p`() BEGIN SELECT 1; END ;;
DELIMITER ;
/*!40103 SET TIME_ZONE=@OLD_TIME_ZONE */;

-- Dump completed
";

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "qadren-split-{name}-{}-{nanos}",
            std::process::id()
        ))
    }

    fn read(dir: &Path, path: &str) -> String {
        std::fs::read_to_string(dir.join(path)).unwrap()
    }

    #[test]
    fn split_keeps_every_line_once_in_load_order() {
        let dir = temp_dir("lines");
        let files = split_dump(DUMP.as_bytes(), &dir).unwrap();
        let paths: Vec<&str> = files.iter().map(|file| file.path.as_str()).collect();
        assert_eq!(paths.first(), Some(&SCHEMA_FILE));
        assert_eq!(paths.last(), Some(&OBJECTS_FILE));
        assert_eq!(paths.len(), 5);
        // Each line of the dump is in one file. Table files add only the
        // header settings and their restore lines.
        let mut source: Vec<&str> = DUMP.lines().collect();
        let mut split = Vec::new();
        for file in &files {
            let text = read(&dir, &file.path);
            let lines: Vec<String> = text.lines().map(str::to_owned).collect();
            if file.table.is_some() {
                let start = lines
                    .iter()
                    .position(|line| line.starts_with("-- Dumping data for table"))
                    .unwrap();
                let end = lines
                    .iter()
                    .rposition(|line| line == "UNLOCK TABLES;")
                    .unwrap();
                split.extend(lines[start..=end].iter().cloned());
            } else {
                split.extend(lines);
            }
            // The recorded size and MD5 are the bytes on disk.
            assert_eq!(file.bytes, text.len() as u64);
            assert_eq!(file.md5, format!("{:x}", md5::compute(text.as_bytes())));
        }
        let mut split: Vec<&str> = split.iter().map(String::as_str).collect();
        source.sort_unstable();
        split.sort_unstable();
        assert_eq!(source, split);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn split_puts_objects_after_the_data() {
        let dir = temp_dir("objects");
        split_dump(DUMP.as_bytes(), &dir).unwrap();
        let schema = read(&dir, SCHEMA_FILE);
        let objects = read(&dir, OBJECTS_FILE);
        assert!(schema.contains("CREATE TABLE `orders`"));
        assert!(schema.contains("CREATE VIEW `v` AS SELECT 1"));
        assert!(!schema.contains("INSERT INTO"));
        assert!(!schema.contains("TRIGGER"));
        assert!(objects.contains("TRIGGER `orders_ai`"));
        assert!(objects.contains("CREATE VIEW `v` AS select 1"));
        assert!(objects.contains("CREATE PROCEDURE `p`"));
        assert!(!objects.contains("INSERT INTO"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn table_file_has_the_session_settings_and_their_restore() {
        let dir = temp_dir("settings");
        split_dump(DUMP.as_bytes(), &dir).unwrap();
        let table = read(&dir, "tables/orders.sql");
        assert!(table.starts_with("/*!40101 SET @OLD_CHARACTER_SET_CLIENT"));
        assert!(table.contains("SET TIME_ZONE='+00:00'"));
        assert!(table.contains("INSERT INTO `orders` VALUES (1),(2);"));
        assert!(!table.contains("TRIGGER"));
        assert!(table.ends_with(
            "UNLOCK TABLES;\n/*!40103 SET TIME_ZONE=@OLD_TIME_ZONE */;\n\
             /*!40101 SET CHARACTER_SET_CLIENT=@OLD_CHARACTER_SET_CLIENT */;\n"
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn table_names_give_safe_and_separate_file_names() {
        let dir = temp_dir("names");
        let files = split_dump(DUMP.as_bytes(), &dir).unwrap();
        let tables: Vec<(&str, &str)> = files
            .iter()
            .filter_map(|file| Some((file.table.as_deref()?, file.path.as_str())))
            .collect();
        assert_eq!(tables[0], ("orders", "tables/orders.sql"));
        // `Orders` is the same name as `orders` on a macOS volume.
        assert_eq!(tables[1].0, "Orders");
        assert!(tables[1].1.starts_with("tables/t-"));
        // A name with a back quote and `/` never becomes a path.
        assert_eq!(tables[2].0, "we`ird/name");
        assert!(tables[2].1.starts_with("tables/t-") && !tables[2].1[7..].contains('/'));
        assert_ne!(tables[1].1, tables[2].1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_table_with_two_data_sections_fails() {
        let dir = temp_dir("twice");
        let dump = "-- Dumping data for table `a`\nINSERT INTO `a` VALUES (1);\nUNLOCK TABLES;\n\
                    -- Dumping data for table `a`\nINSERT INTO `a` VALUES (2);\nUNLOCK TABLES;\n";
        let error = split_dump(dump.as_bytes(), &dir).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn manifest_is_the_same_for_the_same_files() {
        let (one, two) = (temp_dir("m1"), temp_dir("m2"));
        let first = split_dump(DUMP.as_bytes(), &one).unwrap();
        let second = split_dump(DUMP.as_bytes(), &two).unwrap();
        assert_eq!(
            split_manifest("example", &first),
            split_manifest("example", &second)
        );
        let manifest: serde_json::Value =
            serde_json::from_slice(&split_manifest("example", &first)).unwrap();
        assert_eq!(
            manifest["load_order"].as_array().unwrap().len(),
            first.len()
        );
        let _ = std::fs::remove_dir_all(&one);
        let _ = std::fs::remove_dir_all(&two);
    }
}
