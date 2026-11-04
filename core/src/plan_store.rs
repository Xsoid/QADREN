//! Stored, streaming Scenario plans (D-024, stage 4).
//!
//! The Worker builds a plan from two complete snapshots and the confirmed
//! file states of the Scenario revision. It merges the three sources in
//! name-key order (NFC, lowercase) through one sorted SQLite query, so names
//! that can name one file are adjacent. Each name-key group is decided with
//! `scenario_planner::plan_path`, the same rule as the in-memory planner,
//! and the rows are written to `plan_items`. Memory does not grow with the
//! number of files. The simple deletion rule is applied after the pass from
//! a database count.
//!
//! The tables live in `snapshots.sqlite3`. The Worker is the only writer.
use crate::model::{
    ConfirmedFileVersion, DeletionMode, FileSyncState, ScenarioReference, ScenarioScope,
    ScopeMatch, SyncDirection,
};
use crate::registry::Registry;
use crate::scenario_planner::{
    name_key, plan_path, ObservedFile, PathObservation, PlanAction, SideChange,
};
use crate::snapshot::{SnapshotError, SnapshotStore};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// Baseline rows copied in one Registry read.
const BASELINE_PAGE: usize = 5_000;

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs() as i64)
}

pub type Result<T> = std::result::Result<T, SnapshotError>;

/// Create the plan tables. Called by the Worker with a writable store.
pub fn ensure_schema(db: &Connection) -> Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS plans (
            id TEXT PRIMARY KEY NOT NULL,
            scenario_id TEXT NOT NULL,
            revision INTEGER NOT NULL,
            left_snapshot TEXT NOT NULL,
            right_snapshot TEXT NOT NULL,
            state TEXT NOT NULL CHECK(state IN ('waiting','building','complete','failed')),
            created_at INTEGER NOT NULL,
            finished_at INTEGER,
            error TEXT,
            counts TEXT
         );
         CREATE INDEX IF NOT EXISTS plans_by_scenario ON plans(scenario_id, revision, created_at);
         CREATE TABLE IF NOT EXISTS plan_baselines (
            plan_id TEXT NOT NULL,
            relative_path TEXT NOT NULL,
            name_key TEXT NOT NULL,
            left_revision TEXT, left_size INTEGER, left_md5 TEXT,
            right_revision TEXT, right_size INTEGER, right_md5 TEXT,
            PRIMARY KEY(plan_id, relative_path)
         ) WITHOUT ROWID;
         CREATE INDEX IF NOT EXISTS plan_baselines_by_key ON plan_baselines(plan_id, name_key);
         CREATE TABLE IF NOT EXISTS plan_items (
            plan_id TEXT NOT NULL,
            seq INTEGER NOT NULL,
            relative_path TEXT NOT NULL,
            action TEXT NOT NULL,
            from_left INTEGER,
            on_left INTEGER,
            reason TEXT,
            left_change TEXT NOT NULL,
            right_change TEXT NOT NULL,
            left_size INTEGER, left_revision TEXT, left_md5 TEXT, left_special INTEGER,
            right_size INTEGER, right_revision TEXT, right_md5 TEXT, right_special INTEGER,
            PRIMARY KEY(plan_id, seq)
         ) WITHOUT ROWID;
         CREATE INDEX IF NOT EXISTS plan_items_by_action ON plan_items(plan_id, action, seq);",
    )?;
    Ok(())
}

/// One stored plan.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct PlanInfo {
    pub id: String,
    pub left_snapshot: String,
    pub right_snapshot: String,
    pub state: String,
    pub created_at: i64,
    pub finished_at: Option<i64>,
    pub error: Option<String>,
    /// For each action kind: `count` and `bytes` (size on the source side).
    pub counts: Option<Value>,
}

fn side_label(change: SideChange) -> &'static str {
    match change {
        SideChange::Unchanged => "unchanged",
        SideChange::Added => "added",
        SideChange::Removed => "removed",
        SideChange::Changed => "changed",
        SideChange::NeedsVerification => "needs_verification",
    }
}

/// The plan of a Scenario revision that waits or is being built, if any.
pub fn active_plan(db: &Connection, reference: &ScenarioReference) -> Result<Option<String>> {
    ensure_schema(db)?;
    Ok(db
        .query_row(
            "SELECT id FROM plans WHERE scenario_id=?1 AND revision=?2
               AND state IN ('waiting','building') ORDER BY created_at DESC LIMIT 1",
            params![reference.scenario_id, reference.revision as i64],
            |row| row.get(0),
        )
        .optional()?)
}

/// Register a plan that waits for its two snapshots.
pub fn request_plan(
    db: &Connection,
    reference: &ScenarioReference,
    left_snapshot: &str,
    right_snapshot: &str,
) -> Result<String> {
    ensure_schema(db)?;
    let waiting: Option<String> = db
        .query_row(
            "SELECT id FROM plans WHERE scenario_id=?1 AND revision=?2 AND left_snapshot=?3
               AND right_snapshot=?4 AND state IN ('waiting','building')",
            params![
                reference.scenario_id,
                reference.revision as i64,
                left_snapshot,
                right_snapshot
            ],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(id) = waiting {
        return Ok(id);
    }
    let id: String = db.query_row("SELECT lower(hex(randomblob(16)))", [], |row| row.get(0))?;
    db.execute(
        "INSERT INTO plans(id,scenario_id,revision,left_snapshot,right_snapshot,state,created_at)
         VALUES(?1,?2,?3,?4,?5,'waiting',?6)",
        params![
            id,
            reference.scenario_id,
            reference.revision as i64,
            left_snapshot,
            right_snapshot,
            unix_now()
        ],
    )?;
    Ok(id)
}

/// Plans that wait for snapshots, with their Scenario revision.
pub fn waiting_plans(db: &Connection) -> Result<Vec<(String, ScenarioReference, String, String)>> {
    ensure_schema(db)?;
    let mut statement = db.prepare(
        "SELECT id,scenario_id,revision,left_snapshot,right_snapshot FROM plans
         WHERE state='waiting' ORDER BY created_at",
    )?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                ScenarioReference {
                    scenario_id: row.get(1)?,
                    revision: row.get::<_, i64>(2)? as u64,
                },
                row.get(3)?,
                row.get(4)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn fail_plan(db: &Connection, id: &str, error: &str) -> Result<()> {
    db.execute(
        "UPDATE plans SET state='failed', error=?2, finished_at=?3 WHERE id=?1",
        params![id, error, unix_now()],
    )?;
    Ok(())
}

pub fn plan_info(db: &Connection, id: &str) -> Result<Option<PlanInfo>> {
    let row = db
        .query_row(
            "SELECT id,left_snapshot,right_snapshot,state,created_at,finished_at,error,counts
             FROM plans WHERE id=?1",
            [id],
            |row| {
                let counts: Option<String> = row.get(7)?;
                Ok(PlanInfo {
                    id: row.get(0)?,
                    left_snapshot: row.get(1)?,
                    right_snapshot: row.get(2)?,
                    state: row.get(3)?,
                    created_at: row.get(4)?,
                    finished_at: row.get(5)?,
                    error: row.get(6)?,
                    counts: counts.and_then(|text| serde_json::from_str(&text).ok()),
                })
            },
        )
        .optional()?;
    Ok(row)
}

/// The newest plan of a Scenario revision.
pub fn latest_plan(db: &Connection, reference: &ScenarioReference) -> Result<Option<PlanInfo>> {
    let id: Option<String> = db
        .query_row(
            "SELECT id FROM plans WHERE scenario_id=?1 AND revision=?2
             ORDER BY created_at DESC, rowid DESC LIMIT 1",
            params![reference.scenario_id, reference.revision as i64],
            |row| row.get(0),
        )
        .optional()
        .or_else(|error| match error {
            rusqlite::Error::SqliteFailure(..) => Ok(None),
            other => Err(other),
        })?;
    match id {
        Some(id) => plan_info(db, &id),
        None => Ok(None),
    }
}

fn version(
    size: Option<i64>,
    revision: Option<String>,
    md5: Option<String>,
) -> Option<ConfirmedFileVersion> {
    size.map(|size| ConfirmedFileVersion {
        object_revision: revision,
        size: size.max(0) as u64,
        checksum_md5: md5,
    })
}

#[derive(Default)]
struct GroupPath {
    left: Option<(ObservedFile, bool)>,
    right: Option<(ObservedFile, bool)>,
    confirmed: Option<FileSyncState>,
}

/// Build a waiting plan: copy the baselines, stream the sorted merge, decide
/// each name-key group, and store the rows. One transaction; a crash leaves
/// the plan `building` and the Worker builds it again from the start.
pub fn build_plan(store_path: &std::path::Path, registry: &Registry, id: &str) -> Result<PlanInfo> {
    let db = SnapshotStore::open(store_path)?.into_connection();
    ensure_schema(&db)?;
    db.execute_batch("PRAGMA temp_store=FILE;")?;
    let (reference, left_snapshot, right_snapshot): (ScenarioReference, String, String) = db
        .query_row(
            "SELECT scenario_id,revision,left_snapshot,right_snapshot FROM plans WHERE id=?1",
            [id],
            |row| {
                Ok((
                    ScenarioReference {
                        scenario_id: row.get(0)?,
                        revision: row.get::<_, i64>(1)? as u64,
                    },
                    row.get(2)?,
                    row.get(3)?,
                ))
            },
        )?;
    let (direction, scope) = registry
        .scenario_plan_inputs(&reference)
        .map_err(|_| SnapshotError::Blocked("scenario_unavailable".into()))?;
    let scope = ScenarioScope {
        selected_paths: ScenarioScope::normalized_selection(&scope.selected_paths)
            .map_err(|_| SnapshotError::Blocked("invalid_selection".into()))?,
        deletion_mode: scope.deletion_mode,
        excluded_paths: scope.excluded_paths,
    };
    let tx = db.unchecked_transaction()?;
    tx.execute("UPDATE plans SET state='building' WHERE id=?1", [id])?;
    tx.execute("DELETE FROM plan_items WHERE plan_id=?1", [id])?;
    tx.execute("DELETE FROM plan_baselines WHERE plan_id=?1", [id])?;
    // 1. Copy the confirmed file states with their name keys, page by page.
    {
        let mut insert = tx.prepare_cached(
            "INSERT OR REPLACE INTO plan_baselines(plan_id,relative_path,name_key,
                left_revision,left_size,left_md5,right_revision,right_size,right_md5)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        )?;
        let mut after = String::new();
        loop {
            let page = registry
                .confirmed_file_states_page(&reference, &after, BASELINE_PAGE)
                .map_err(|_| SnapshotError::Transient("registry_unavailable".into()))?;
            let Some(last) = page.last() else { break };
            after = last.relative_path.clone();
            for state in &page {
                let (l, r) = (state.left.as_ref(), state.right.as_ref());
                insert.execute(params![
                    id,
                    state.relative_path,
                    name_key(&state.relative_path),
                    l.and_then(|v| v.object_revision.clone()),
                    l.map(|v| v.size as i64),
                    l.and_then(|v| v.checksum_md5.clone()),
                    r.and_then(|v| v.object_revision.clone()),
                    r.map(|v| v.size as i64),
                    r.and_then(|v| v.checksum_md5.clone()),
                ])?;
            }
        }
    }
    // 2. Stream the merge in name-key order and decide each group.
    let mut seq: i64 = 0;
    {
        let mut insert = tx.prepare_cached(
            "INSERT INTO plan_items(plan_id,seq,relative_path,action,from_left,on_left,reason,
                left_change,right_change,left_size,left_revision,left_md5,left_special,
                right_size,right_revision,right_md5,right_special)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17)",
        )?;
        let mut statement = tx.prepare(
            "SELECT name_key,relative_path,0,size,revision,md5,special,NULL,NULL,NULL
               FROM snapshot_files WHERE snapshot_id=?1
             UNION ALL
             SELECT name_key,relative_path,1,size,revision,md5,special,NULL,NULL,NULL
               FROM snapshot_files WHERE snapshot_id=?2
             UNION ALL
             SELECT name_key,relative_path,2,left_size,left_revision,left_md5,0,
                    right_size,right_revision,right_md5
               FROM plan_baselines WHERE plan_id=?3
             ORDER BY 1,2",
        )?;
        let mut rows = statement.query(params![left_snapshot, right_snapshot, id])?;
        let mut current_key: Option<String> = None;
        let mut group: BTreeMap<String, GroupPath> = BTreeMap::new();
        let mut flush = |group: &mut BTreeMap<String, GroupPath>, seq: &mut i64| -> Result<()> {
            let in_scope: Vec<_> = std::mem::take(group)
                .into_iter()
                .filter(|(path, _)| scope.classify(path, name_key) != ScopeMatch::Outside)
                .collect();
            let collides = in_scope.len() > 1;
            for (path, entry) in in_scope {
                let skipped = entry.left.as_ref().is_some_and(|(_, special)| *special)
                    || entry.right.as_ref().is_some_and(|(_, special)| *special);
                let left = entry
                    .left
                    .as_ref()
                    .filter(|(_, special)| !*special)
                    .map(|(file, _)| file);
                let right = entry
                    .right
                    .as_ref()
                    .filter(|(_, special)| !*special)
                    .map(|(file, _)| file);
                let item = plan_path(
                    direction,
                    &scope,
                    &PathObservation {
                        path: &path,
                        confirmed: entry.confirmed.as_ref(),
                        left,
                        right,
                        skipped,
                    },
                    collides,
                );
                let (kind, from_left, on_left, reason) = match item.action {
                    PlanAction::NoAction => ("no_action", None, None, None),
                    PlanAction::Create { from_left } => ("create", Some(from_left), None, None),
                    PlanAction::Update { from_left } => ("update", Some(from_left), None, None),
                    PlanAction::DeleteRequiresDecision { on_left } => {
                        ("delete_requires_decision", None, Some(on_left), None)
                    }
                    PlanAction::Delete { on_left } => ("delete", None, Some(on_left), None),
                    PlanAction::Conflict { reason } => ("conflict", None, None, Some(reason)),
                    PlanAction::NeedsVerification { reason } => {
                        ("needs_verification", None, None, Some(reason))
                    }
                };
                *seq += 1;
                let l = entry.left.as_ref();
                let r = entry.right.as_ref();
                insert.execute(params![
                    id,
                    *seq,
                    path,
                    kind,
                    from_left,
                    on_left,
                    reason,
                    side_label(item.left),
                    side_label(item.right),
                    l.map(|(f, _)| f.size as i64),
                    l.and_then(|(f, _)| f.object_revision.clone()),
                    l.and_then(|(f, _)| f.checksum_md5.clone()),
                    l.map(|(_, special)| *special),
                    r.map(|(f, _)| f.size as i64),
                    r.and_then(|(f, _)| f.object_revision.clone()),
                    r.and_then(|(f, _)| f.checksum_md5.clone()),
                    r.map(|(_, special)| *special),
                ])?;
            }
            Ok(())
        };
        while let Some(row) = rows.next()? {
            let key: String = row.get(0)?;
            let path: String = row.get(1)?;
            let source: i64 = row.get(2)?;
            if current_key.as_deref() != Some(key.as_str()) {
                flush(&mut group, &mut seq)?;
                current_key = Some(key);
            }
            let entry = group.entry(path.clone()).or_default();
            match source {
                0 | 1 => {
                    let file = ObservedFile {
                        relative_path: path.clone(),
                        object_revision: row.get(4)?,
                        size: row.get::<_, Option<i64>>(3)?.unwrap_or(0).max(0) as u64,
                        checksum_md5: row.get(5)?,
                    };
                    let special: bool = row.get(6)?;
                    if source == 0 {
                        entry.left = Some((file, special));
                    } else {
                        entry.right = Some((file, special));
                    }
                }
                _ => {
                    entry.confirmed = Some(FileSyncState {
                        scenario: reference.clone(),
                        relative_path: path,
                        left: version(row.get(3)?, row.get(4)?, row.get(5)?),
                        right: version(row.get(7)?, row.get(8)?, row.get(9)?),
                    });
                }
            }
        }
        flush(&mut group, &mut seq)?;
    }
    // 3. The simple deletion rule, with no mass-deletion threshold (D-025).
    if scope.deletion_mode == DeletionMode::Simple {
        tx.execute(
            "UPDATE plan_items SET action='delete' WHERE plan_id=?1 AND action='delete_requires_decision'",
            [id],
        )?;
    }
    // 4. Counts and bytes for each action, from the source side.
    let source_left = !matches!(direction, SyncDirection::RightToLeft);
    let counts = {
        let mut statement = tx.prepare(
            "SELECT action, count(*),
                    coalesce(sum(CASE WHEN coalesce(from_left, ?2)=1 THEN left_size ELSE right_size END),0)
             FROM plan_items WHERE plan_id=?1 GROUP BY action",
        )?;
        let rows = statement
            .query_map(params![id, source_left], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut map = serde_json::Map::new();
        for (action, count, bytes) in rows {
            map.insert(action, json!({"count": count, "bytes": bytes}));
        }
        Value::Object(map)
    };
    tx.execute("DELETE FROM plan_baselines WHERE plan_id=?1", [id])?;
    tx.execute(
        "UPDATE plans SET state='complete', finished_at=?2, counts=?3 WHERE id=?1",
        params![id, unix_now(), counts.to_string()],
    )?;
    // Waiting plans of this revision that are older than this one are
    // superseded by it.
    tx.execute(
        "UPDATE plans SET state='failed', error='superseded', finished_at=?3
         WHERE scenario_id=?1 AND revision=?2 AND state='waiting' AND created_at<=(
            SELECT created_at FROM plans WHERE id=?4) AND id!=?4",
        params![
            reference.scenario_id,
            reference.revision as i64,
            unix_now(),
            id
        ],
    )?;
    // Keep only this plan of the Scenario revision; plans are large.
    tx.execute(
        "DELETE FROM plan_items WHERE plan_id IN (
            SELECT id FROM plans WHERE scenario_id=?1 AND revision=?2 AND state!='waiting'
            ORDER BY created_at DESC LIMIT -1 OFFSET 1)",
        params![reference.scenario_id, reference.revision as i64],
    )?;
    tx.execute(
        "DELETE FROM plans WHERE id IN (
            SELECT id FROM plans WHERE scenario_id=?1 AND revision=?2 AND state!='waiting'
            ORDER BY created_at DESC LIMIT -1 OFFSET 1)",
        params![reference.scenario_id, reference.revision as i64],
    )?;
    tx.commit()?;
    let _ = crate::snapshot::release_free_pages(&db);
    plan_info(&db, id)?.ok_or(SnapshotError::NotFound)
}

/// Remove the plans of Scenarios that no longer exist in the Registry.
/// A removed Scenario never needs its plans again; the snapshot prune then
/// frees the snapshots that only these plans kept.
pub fn remove_plans_of_removed_scenarios(db: &Connection, registry: &Registry) -> Result<usize> {
    let existing: std::collections::HashSet<String> = registry
        .list_scenarios()
        .map_err(|_| SnapshotError::Blocked("registry_unavailable".into()))?
        .into_iter()
        .map(|scenario| scenario.id)
        .collect();
    let stored: Vec<String> = db
        .prepare("SELECT DISTINCT scenario_id FROM plans")?
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let mut removed = 0;
    for scenario_id in stored.iter().filter(|id| !existing.contains(*id)) {
        let tx = db.unchecked_transaction()?;
        for table in ["plan_items", "plan_baselines"] {
            tx.execute(
                &format!("DELETE FROM {table} WHERE plan_id IN (SELECT id FROM plans WHERE scenario_id=?1)"),
                [scenario_id],
            )?;
        }
        removed += tx.execute("DELETE FROM plans WHERE scenario_id=?1", [scenario_id])?;
        tx.commit()?;
    }
    Ok(removed)
}

/// One page of plan rows in plan order, in the JSON shape of the app plan
/// items, with the file metadata of both sides. `action` filters by kind;
/// `search` keeps rows whose path contains it.
pub fn plan_page(
    db: &Connection,
    id: &str,
    action: Option<&str>,
    search: Option<&str>,
    offset: u64,
    limit: u64,
) -> Result<Vec<Value>> {
    let pattern = search.filter(|text| !text.is_empty()).map(|text| {
        format!(
            "%{}%",
            text.replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_")
        )
    });
    let mut statement = db.prepare(
        "SELECT relative_path,action,from_left,on_left,reason,left_change,right_change,
                left_size,left_revision,left_md5,left_special,
                right_size,right_revision,right_md5,right_special
         FROM plan_items
         WHERE plan_id=?1 AND (?2 IS NULL OR action=?2)
           AND (?3 IS NULL OR relative_path LIKE ?3 ESCAPE '\\')
         ORDER BY seq LIMIT ?4 OFFSET ?5",
    )?;
    let rows = statement
        .query_map(
            params![id, action, pattern, limit.min(1_000) as i64, offset as i64],
            |row| {
                let path: String = row.get(0)?;
                let file = |size: Option<i64>,
                            revision: Option<String>,
                            md5: Option<String>,
                            special: Option<bool>| {
                    size.map(|size| {
                        json!({
                            "relative_path": path,
                            "object_revision": revision,
                            "size": size,
                            "checksum_md5": md5,
                            "special": special.unwrap_or(false),
                        })
                    })
                };
                Ok(json!({
                    "relative_path": path,
                    "action": {
                        "kind": row.get::<_, String>(1)?,
                        "from_left": row.get::<_, Option<bool>>(2)?,
                        "on_left": row.get::<_, Option<bool>>(3)?,
                        "reason": row.get::<_, Option<String>>(4)?,
                    },
                    "left": row.get::<_, String>(5)?,
                    "right": row.get::<_, String>(6)?,
                    "left_file": file(row.get(7)?, row.get(8)?, row.get(9)?, row.get(10)?),
                    "right_file": file(row.get(11)?, row.get(12)?, row.get(13)?, row.get(14)?),
                }))
            },
        )?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// One page of the executable rows after `after_seq` (D-025 section 6).
/// Returns the rows of the listed actions as (path, action, source_is_left)
/// and the last `seq` read, or `None` at the end. `action` is `create`,
/// `update`, or `delete`. For a Delete, `source_is_left` names the side
/// whose object is absent.
#[allow(clippy::type_complexity)]
pub fn plan_effects_page(
    db: &Connection,
    id: &str,
    actions: &[&str],
    after_seq: i64,
    limit: usize,
) -> Result<(Vec<(String, String, bool)>, Option<i64>)> {
    let mut statement = db.prepare(
        "SELECT seq,relative_path,action,from_left,on_left FROM plan_items
         WHERE plan_id=?1 AND action IN ('create','update','delete') AND seq>?2
         ORDER BY seq LIMIT ?3",
    )?;
    let mut rows = Vec::new();
    let mut last = None;
    let mut query = statement.query(params![id, after_seq, limit as i64])?;
    while let Some(row) = query.next()? {
        last = Some(row.get::<_, i64>(0)?);
        let action: String = row.get(2)?;
        if !actions.contains(&action.as_str()) {
            continue;
        }
        let source_is_left = if action == "delete" {
            !row.get::<_, Option<bool>>(4)?.unwrap_or(false)
        } else {
            row.get::<_, Option<bool>>(3)?.unwrap_or(true)
        };
        rows.push((row.get(1)?, action, source_is_left));
    }
    Ok((rows, last))
}

/// One page of the Create rows after `after_seq`: (seq, path, from_left).
pub fn plan_creates_page(
    db: &Connection,
    id: &str,
    after_seq: i64,
    limit: usize,
) -> Result<Vec<(i64, String, bool)>> {
    let mut statement = db.prepare(
        "SELECT seq,relative_path,from_left FROM plan_items
         WHERE plan_id=?1 AND action='create' AND seq>?2 ORDER BY seq LIMIT ?3",
    )?;
    let rows = statement
        .query_map(params![id, after_seq, limit as i64], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get::<_, Option<bool>>(2)?.unwrap_or(true),
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}
