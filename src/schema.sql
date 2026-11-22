CREATE TABLE IF NOT EXISTS panel_log_archive (
    host TEXT PRIMARY KEY NOT NULL,
    cursor TEXT
);
CREATE TABLE IF NOT EXISTS panel_attack_sources (
    host TEXT NOT NULL,
    ip TEXT NOT NULL,
    asn TEXT NOT NULL,
    network TEXT NOT NULL,
    count INTEGER NOT NULL,
    last TEXT NOT NULL,
    PRIMARY KEY(host,ip)
);
CREATE TABLE IF NOT EXISTS panel_metrics (
    host TEXT PRIMARY KEY NOT NULL,
    data TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS panel_versions (
    host TEXT PRIMARY KEY NOT NULL,
    branch TEXT NOT NULL,
    latest TEXT
);
CREATE TABLE IF NOT EXISTS metric_history (
    host TEXT NOT NULL,
    time INTEGER NOT NULL,
    cpu REAL,
    ram REAL,
    rx REAL,
    tx REAL
);
CREATE INDEX IF NOT EXISTS metric_history_host_time ON metric_history(host,time);
CREATE TABLE IF NOT EXISTS polls (
    host TEXT NOT NULL,
    time INTEGER NOT NULL,
    status TEXT NOT NULL,
    detail TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS polls_host_time ON polls(host, time);
CREATE INDEX IF NOT EXISTS polls_time ON polls(time);
CREATE TABLE IF NOT EXISTS samples (
    host TEXT NOT NULL,
    time INTEGER NOT NULL,
    family TEXT NOT NULL,
    boot_id TEXT NOT NULL,
    packets INTEGER NOT NULL,
    bytes INTEGER NOT NULL,
    dropped_packets INTEGER,
    dropped_bytes INTEGER
);
CREATE INDEX IF NOT EXISTS samples_host_family_time ON samples(host, family, time);
CREATE INDEX IF NOT EXISTS samples_host_time ON samples(host, time);
CREATE INDEX IF NOT EXISTS samples_time ON samples(time);
CREATE TABLE IF NOT EXISTS cursors (
    host TEXT NOT NULL,
    family TEXT NOT NULL,
    cursor TEXT NOT NULL,
    PRIMARY KEY (host, family)
);
CREATE TABLE IF NOT EXISTS events (
    host TEXT NOT NULL,
    time INTEGER NOT NULL,
    source TEXT NOT NULL,
    protocol TEXT NOT NULL,
    port INTEGER,
    raw TEXT
);
CREATE INDEX IF NOT EXISTS events_time ON events(time);
CREATE INDEX IF NOT EXISTS events_host_time ON events(host, time);
CREATE INDEX IF NOT EXISTS events_host_source_time ON events(host, source, time);
CREATE TABLE IF NOT EXISTS source_stats (
    host TEXT NOT NULL,
    source TEXT NOT NULL,
    asn TEXT,
    netname TEXT,
    logged_count INTEGER,
    last_packet TEXT,
    seen INTEGER NOT NULL,
    PRIMARY KEY (host, source)
);
CREATE TABLE IF NOT EXISTS network_lookup (
    source TEXT PRIMARY KEY NOT NULL,
    prefix TEXT NOT NULL,
    asn TEXT NOT NULL,
    netname TEXT NOT NULL,
    provider TEXT NOT NULL DEFAULT 'RIPEstat',
    seen INTEGER NOT NULL
);
