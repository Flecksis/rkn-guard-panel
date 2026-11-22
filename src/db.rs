use anyhow::{Result, ensure};
use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub struct Host {
    pub name: String,
    pub addr: String,
    pub port: u16,
    pub host_key: String,
    pub secret: Vec<u8>,
    pub active: bool,
}
#[derive(Serialize, Deserialize, Default)]
pub struct Snapshot {
    pub version: String,
    pub firewall: String,
    pub boot_id: String,
    pub cursor: String,
    pub families: Vec<Counter>,
    pub events: Vec<Event>,
    #[serde(default)]
    pub system: Option<SystemMetrics>,
    #[serde(default)]
    pub version_info: Option<VersionInfo>,
    #[serde(default)]
    pub attack_sources: Option<Vec<AttackSource>>,
    #[serde(default)]
    pub log_archive: Option<LogArchive>,
}
#[derive(Serialize, Deserialize)]
pub struct LogArchive {
    pub cursor: Option<String>,
    events: Vec<Event>,
}
#[derive(Serialize, Deserialize)]
pub struct AttackSource {
    ip: String,
    asn: String,
    network: String,
    count: i64,
    last: String,
}
#[derive(Serialize, Deserialize)]
pub struct VersionInfo {
    branch: String,
    latest: Option<String>,
}
#[derive(Serialize, Deserialize)]
pub struct SystemMetrics {
    pub uptime: f64,
    pub cores: Option<u32>,
    pub load: [f64; 3],
    pub cpu_total: u64,
    pub cpu_idle: u64,
    pub mem_total: u64,
    pub mem_used: u64,
    pub disk_total: u64,
    pub disk_used: u64,
    pub interface: Option<String>,
    pub rx: Option<u64>,
    pub tx: Option<u64>,
}

#[derive(Serialize, Deserialize)]
pub struct NodeMetrics {
    boot_id: String,
    pub system: SystemMetrics,
    pub cpu: Option<f64>,
    pub rx_rate: Option<f64>,
    pub tx_rate: Option<f64>,
}

pub fn metrics(db: &Connection, host: &str) -> Result<Option<NodeMetrics>> {
    let raw: Option<String> = db
        .query_row("SELECT data FROM panel_metrics WHERE host=?", [host], |r| {
            r.get(0)
        })
        .optional()?;
    Ok(raw.map(|data| serde_json::from_str(&data)).transpose()?)
}
#[derive(Serialize, Deserialize)]
pub struct Counter {
    pub family: String,
    pub packets: i64,
    pub bytes: i64,
}
#[derive(Serialize, Deserialize)]
pub struct Event {
    pub time: i64,
    pub source: String,
    pub protocol: String,
    pub port: Option<u16>,
    pub raw: String,
}

pub fn path() -> Result<PathBuf> {
    let exe = std::env::current_exe()?;
    let base = exe.parent().unwrap();
    let previous = base.join("rkn-fleet.sqlite3");
    if previous.is_file() {
        return Ok(previous);
    }
    let dir = base.join("data");
    std::fs::create_dir_all(&dir)?;
    Ok(dir.join("rkn-fleet.sqlite3"))
}
pub fn open(path: &Path) -> Result<Connection> {
    let db = Connection::open(path)?;
    db.busy_timeout(std::time::Duration::from_secs(5))?;
    db.execute_batch(include_str!("schema.sql"))?;
    if !db.prepare("SELECT raw FROM events LIMIT 0").is_ok() {
        db.execute("ALTER TABLE events ADD COLUMN raw TEXT", [])?;
    }
    db.execute_batch("PRAGMA journal_mode=WAL;
    CREATE TABLE IF NOT EXISTS panel_hosts(name TEXT PRIMARY KEY, address TEXT, port INTEGER, host_key TEXT, secret BLOB, active INTEGER, UNIQUE(address,port));
    CREATE TABLE IF NOT EXISTS panel_state(host TEXT PRIMARY KEY,cursor TEXT,version TEXT,firewall TEXT);
    CREATE TABLE IF NOT EXISTS panel_settings(key TEXT PRIMARY KEY,value TEXT);")?;
    Ok(db)
}
pub fn hosts(db: &Connection) -> Result<Vec<Host>> {
    let mut q=db.prepare("SELECT name,address,port,host_key,secret,active FROM panel_hosts ORDER BY name COLLATE NOCASE")?;
    Ok(q.query_map([], |r| {
        Ok(Host {
            name: r.get(0)?,
            addr: r.get(1)?,
            port: r.get(2)?,
            host_key: r.get(3)?,
            secret: r.get(4)?,
            active: r.get(5)?,
        })
    })?
    .collect::<rusqlite::Result<_>>()?)
}

pub fn update_profile(
    db: &mut Connection,
    original: &str,
    name: &str,
    address: &str,
    port: u16,
    order: &[&str],
) -> Result<bool> {
    let tx = db.transaction()?;
    ensure!(
        tx.execute(
            "UPDATE panel_hosts SET name=?,address=?,port=? WHERE name=?",
            params![name, address, port, original]
        )? == 1,
        "Профиль сервера не найден"
    );
    if original != name {
        for table in [
            "panel_state",
            "panel_metrics",
            "panel_versions",
            "metric_history",
            "panel_attack_sources",
            "panel_log_archive",
            "events",
            "samples",
            "polls",
            "cursors",
            "source_stats",
        ] {
            let busy: bool = tx.query_row(
                &format!("SELECT EXISTS(SELECT 1 FROM {table} WHERE host=?)"),
                [name],
                |r| r.get(0),
            )?;
            ensure!(!busy, "У другого сервера есть история с таким именем");
            tx.execute(
                &format!("UPDATE {table} SET host=? WHERE host=?"),
                params![name, original],
            )?;
        }
    }
    tx.execute(
        "INSERT OR REPLACE INTO panel_settings VALUES('server_order',?)",
        [serde_json::to_string(order)?],
    )?;
    let active = tx.query_row("SELECT active FROM panel_hosts WHERE name=?", [name], |r| {
        r.get(0)
    })?;
    tx.commit()?;
    Ok(active)
}
pub fn save(db: &mut Connection, h: &Host, s: Snapshot) -> Result<()> {
    ensure!(
        !s.boot_id.is_empty() && !s.families.is_empty(),
        "Неполный замер"
    );
    let tx = db.transaction()?;
    let now = Utc::now().timestamp();
    if let Some(sources) = s.attack_sources {
        tx.execute("DELETE FROM panel_attack_sources WHERE host=?", [&h.name])?;
        for source in sources {
            tx.execute(
                "INSERT OR REPLACE INTO panel_attack_sources VALUES(?,?,?,?,?,?)",
                params![
                    h.name,
                    source.ip,
                    source.asn,
                    source.network,
                    source.count,
                    source.last
                ],
            )?;
        }
    }
    if let Some(system) = s.system {
        let mut current = NodeMetrics {
            boot_id: s.boot_id.clone(),
            system,
            cpu: None,
            rx_rate: None,
            tx_rate: None,
        };
        if let Some(previous) = metrics(&tx, &h.name)?.filter(|p| p.boot_id == current.boot_id) {
            let new = &current.system;
            let old = &previous.system;
            let elapsed = new.uptime - old.uptime;
            if elapsed > 0.0 {
                if let (Some(total), Some(idle)) = (
                    new.cpu_total.checked_sub(old.cpu_total),
                    new.cpu_idle.checked_sub(old.cpu_idle),
                ) {
                    if total > 0 && idle <= total {
                        current.cpu = Some((total - idle) as f64 / total as f64);
                    }
                }
                if new.interface == old.interface {
                    let rate = |after: Option<u64>, before: Option<u64>| {
                        Some(after?.checked_sub(before?)? as f64 / elapsed)
                    };
                    current.rx_rate = rate(new.rx, old.rx);
                    current.tx_rate = rate(new.tx, old.tx);
                }
            }
        }
        tx.execute(
            "INSERT OR REPLACE INTO panel_metrics VALUES(?,?)",
            params![h.name, serde_json::to_string(&current)?],
        )?;
        let ram = (current.system.mem_total > 0)
            .then(|| current.system.mem_used as f64 / current.system.mem_total as f64);
        tx.execute(
            "INSERT INTO metric_history VALUES(?,?,?,?,?,?)",
            params![
                h.name,
                now,
                current.cpu,
                ram,
                current.rx_rate,
                current.tx_rate
            ],
        )?;
        tx.execute(
            "DELETE FROM metric_history WHERE host=? AND time<?",
            params![h.name, now - 2 * 86400],
        )?;
    } else {
        tx.execute("DELETE FROM panel_metrics WHERE host=?", [&h.name])?;
    }
    for f in s.families {
        ensure!(
            matches!(f.family.as_str(), "4" | "6") && f.bytes >= 0 && f.packets >= 0,
            "Некорректный счётчик"
        );
        let prev=tx.query_row("SELECT boot_id,packets,bytes FROM samples WHERE host=? AND family=? ORDER BY rowid DESC LIMIT 1",params![h.name,f.family],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?,r.get::<_,i64>(2)?))).ok();
        let delta = prev
            .filter(|(b, p, v)| b == &s.boot_id && f.packets >= *p && f.bytes >= *v)
            .map(|(_, p, v)| (f.packets - p, f.bytes - v));
        tx.execute(
            "INSERT INTO samples VALUES(?,?,?,?,?,?,?,?)",
            params![
                h.name,
                now,
                f.family,
                s.boot_id,
                f.packets,
                f.bytes,
                delta.map(|v| v.0),
                delta.map(|v| v.1)
            ],
        )?;
    }
    let mut incoming = s.events;
    if let Some(archive) = s.log_archive {
        tx.execute(
            "INSERT OR REPLACE INTO panel_log_archive VALUES(?,?)",
            params![h.name, archive.cursor],
        )?;
        incoming.extend(archive.events);
    }
    {
        let mut insert = tx.prepare(
            "INSERT INTO events(host,time,source,protocol,port,raw)
             SELECT ?1,?2,?3,?4,?5,?6 WHERE NOT EXISTS (
                SELECT 1 FROM events WHERE host=?1 AND time=?2 AND source=?3
                AND protocol=?4 AND port IS ?5 AND raw IS ?6
             )",
        )?;
        for e in incoming {
            insert.execute(params![h.name, e.time, e.source, e.protocol, e.port, e.raw])?;
        }
    }
    tx.execute(
        "INSERT OR REPLACE INTO panel_state VALUES(?,?,?,?)",
        params![h.name, s.cursor, s.version, s.firewall],
    )?;
    if let Some(info) = s.version_info {
        tx.execute(
            "INSERT OR REPLACE INTO panel_versions VALUES(?,?,?)",
            params![h.name, info.branch, info.latest],
        )?;
    } else {
        tx.execute("DELETE FROM panel_versions WHERE host=?", [&h.name])?;
    }
    tx.execute(
        "INSERT INTO polls VALUES(?,?,'ok','')",
        params![h.name, now],
    )?;
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn архив_и_живой_журнал_не_дублируют_запись() -> Result<()> {
        let mut db = open(Path::new(":memory:"))?;
        db.execute(
            "INSERT INTO panel_hosts VALUES('node','192.0.2.1',22,'key',x'A1',1)",
            [],
        )?;
        let host = hosts(&db)?.remove(0);
        let event = serde_json::json!({"time": 100, "source":"192.0.2.4", "protocol":"TCP", "port":443, "raw":"packet"});
        let snapshot = serde_json::json!({
            "version":"dev", "firewall":"iptables", "boot_id":"boot", "cursor":"",
            "families":[{"family":"4","packets":5,"bytes":50}],
            "events":[event.clone()],
            "log_archive":{"cursor":null,"events":[event]},
        });
        save(&mut db, &host, serde_json::from_value(snapshot)?)?;
        assert_eq!(
            db.query_row("SELECT count(*) FROM events", [], |r| r.get::<_, i64>(0))?,
            1
        );
        Ok(())
    }

    #[test]
    fn смена_адреса_и_имени_сохраняет_профиль_и_историю() -> Result<()> {
        let mut db = open(Path::new(":memory:"))?;
        db.execute(
            "INSERT INTO panel_hosts VALUES('old','192.0.2.1',22,'host-key',x'A1',1)",
            [],
        )?;
        db.execute(
            "INSERT INTO metric_history VALUES('old',100,0.5,0.3,NULL,NULL)",
            [],
        )?;
        assert!(update_profile(
            &mut db,
            "old",
            "new",
            "192.0.2.2",
            2222,
            &["new"]
        )?);
        let host = hosts(&db)?.remove(0);
        assert_eq!(
            (host.name.as_str(), host.addr.as_str(), host.port),
            ("new", "192.0.2.2", 2222)
        );
        assert_eq!(host.host_key, "host-key");
        assert_eq!(host.secret, [0xa1]);
        assert_eq!(
            db.query_row("SELECT host FROM metric_history", [], |r| r
                .get::<_, String>(0))?,
            "new"
        );
        Ok(())
    }
}
