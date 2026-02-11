use crate::paths::SeashailPaths;
use eyre::Context as _;

// Local, embedded "SQLite-like" store (Turso, pure Rust).
//
// This is explicitly used for non-critical caching/metadata (e.g. price TTL cache).
// If it fails/corrupts, callers should treat it as best-effort and fall back to live calls.
//
// Also persists best-effort snapshots (e.g. perps positions/market data cache) so read tools
// can return something useful even when a venue/API is temporarily unavailable.

pub struct Db {
    // Keep the database handle alive for the lifetime of the connection.
    _db: turso::Database,
    conn: turso::Connection,
}

#[derive(Debug, Clone)]
pub struct HealthSnapshotRow {
    pub surface: String,
    pub chain: String,
    pub provider: String,
    pub wallet: String,
    pub account_index: i64,
    pub fetched_at_ms: i64,
    pub payload_json: String,
}

#[derive(Debug, Clone)]
pub struct PortfolioSnapshotTotalRow {
    pub snapshot_id: i64,
    pub fetched_at_ms: i64,
    pub day: String,
    pub total_usd: f64,
}

pub struct HealthSnapshotInput<'a> {
    pub surface: &'a str,
    pub chain: &'a str,
    pub provider: &'a str,
    pub wallet: &'a str,
    pub account_index: i64,
    pub fetched_at_ms: i64,
    pub payload_json: &'a str,
}

fn parse_env_bool(name: &str) -> Option<bool> {
    let v = std::env::var(name).ok()?;
    match v.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "y" | "on" => Some(true),
        "0" | "false" | "no" | "n" | "off" => Some(false),
        _ => None,
    }
}

// `turso::Database` / `turso::Connection` may not implement `Debug`. We only need a
// debuggable handle for state struct derives, not to print internals.
impl std::fmt::Debug for Db {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Db").finish_non_exhaustive()
    }
}

#[derive(Debug, Clone)]
pub struct CachedPriceRow {
    pub usd: f64,
    pub source: String,
}

#[derive(Debug, Clone)]
pub struct CachedJsonRow {
    pub json: String,
}

impl Db {
    pub async fn open(paths: &SeashailPaths, default_shared: bool) -> eyre::Result<Self> {
        // The data_dir is already enforced private (0700 on Unix) and symlink-checked.
        crate::fsutil::ensure_private_dir(&paths.data_dir)?;

        // This DB is a best-effort cache. Some embedded DBs are not happy with multiple
        // concurrent writers/processes on the same file. Default to a per-process DB file
        // so multiple `seashail mcp --standalone` processes can run without blocking.
        //
        // The singleton daemon defaults to a stable shared DB file. `SEASHAIL_SHARED_DB`
        // overrides either default.
        let shared = parse_env_bool("SEASHAIL_SHARED_DB").unwrap_or(default_shared);
        let p = if shared {
            paths.data_dir.join("seashail.db")
        } else {
            paths
                .data_dir
                .join(format!("seashail.{}.db", std::process::id()))
        };
        let p_s = p.to_string_lossy();

        let db = turso::Builder::new_local(p_s.as_ref())
            .build()
            .await
            .context("open turso local db")?;
        let conn = db.connect().context("connect turso db")?;

        let this = Self { _db: db, conn };
        this.init().await?;
        Ok(this)
    }

    async fn init(&self) -> eyre::Result<()> {
        // Single-table TTL cache for prices. Key design lives in price.rs.
        self.conn
            .execute(
                "CREATE TABLE IF NOT EXISTS seashail_prices (\
                  key TEXT PRIMARY KEY,\
                  usd REAL NOT NULL,\
                  source TEXT NOT NULL,\
                  fetched_at_ms INTEGER NOT NULL,\
                  stale_at_ms INTEGER NOT NULL\
                )",
                (),
            )
            .await
            .context("create seashail_prices")?;

        // Generic JSON TTL cache for best-effort snapshots (perps positions, venue metadata).
        self.conn
            .execute(
                "CREATE TABLE IF NOT EXISTS seashail_cache_json (\
                  key TEXT PRIMARY KEY,\
                  json TEXT NOT NULL,\
                  fetched_at_ms INTEGER NOT NULL,\
                  stale_at_ms INTEGER NOT NULL\
                )",
                (),
            )
            .await
            .context("create seashail_cache_json")?;

        // Durable position/health snapshots for monitoring.
        //
        // Primary key enforces "latest per surface/provider/wallet/account". This is intentionally
        // best-effort; failures should not break read paths.
        self.conn
            .execute(
                "CREATE TABLE IF NOT EXISTS seashail_health_snapshots (\
                  surface TEXT NOT NULL,\
                  chain TEXT NOT NULL,\
                  provider TEXT NOT NULL,\
                  wallet TEXT NOT NULL,\
                  account_index INTEGER NOT NULL,\
                  fetched_at_ms INTEGER NOT NULL,\
                  payload_json TEXT NOT NULL,\
                  PRIMARY KEY (surface, chain, provider, wallet, account_index)\
                )",
                (),
            )
            .await
            .context("create seashail_health_snapshots")?;

        // Durable portfolio snapshots for historical tracking and P&L deltas.
        self.conn
            .execute(
                "CREATE TABLE IF NOT EXISTS seashail_portfolio_snapshots (\
                  snapshot_id INTEGER PRIMARY KEY AUTOINCREMENT,\
                  fetched_at_ms INTEGER NOT NULL,\
                  day TEXT NOT NULL,\
                  scope_json TEXT NOT NULL\
                )",
                (),
            )
            .await
            .context("create seashail_portfolio_snapshots")?;
        self.conn
            .execute(
                "CREATE TABLE IF NOT EXISTS seashail_portfolio_snapshot_items (\
                  snapshot_id INTEGER NOT NULL,\
                  wallet TEXT NOT NULL,\
                  account_index INTEGER NOT NULL,\
                  chain TEXT NOT NULL,\
                  usd_value REAL NOT NULL,\
                  payload_json TEXT NOT NULL\
                )",
                (),
            )
            .await
            .context("create seashail_portfolio_snapshot_items")?;

        Ok(())
    }

    pub async fn get_price_if_fresh(
        &self,
        key: &str,
        now_ms: i64,
    ) -> eyre::Result<Option<CachedPriceRow>> {
        let mut rows = self
            .conn
            .query(
                "SELECT usd, source, stale_at_ms FROM seashail_prices WHERE key = ?",
                (key,),
            )
            .await
            .context("query seashail_prices")?;

        let Some(row) = rows.next().await.context("next row")? else {
            return Ok(None);
        };

        let usd: f64 = row.get(0).context("row.usd")?;
        let source: String = row.get(1).context("row.source")?;
        let stale_at_ms: i64 = row.get(2).context("row.stale_at_ms")?;

        if now_ms >= stale_at_ms {
            return Ok(None);
        }

        Ok(Some(CachedPriceRow { usd, source }))
    }

    pub async fn upsert_price(
        &self,
        key: &str,
        usd: f64,
        source: &str,
        fetched_at_ms: i64,
        stale_at_ms: i64,
    ) -> eyre::Result<()> {
        self.conn
            .execute(
                "INSERT INTO seashail_prices (key, usd, source, fetched_at_ms, stale_at_ms) \
                 VALUES (?, ?, ?, ?, ?) \
                 ON CONFLICT(key) DO UPDATE SET \
                   usd=excluded.usd, \
                   source=excluded.source, \
                   fetched_at_ms=excluded.fetched_at_ms, \
                   stale_at_ms=excluded.stale_at_ms",
                (key, usd, source, fetched_at_ms, stale_at_ms),
            )
            .await
            .context("upsert seashail_prices")?;
        Ok(())
    }

    pub async fn get_json_if_fresh(
        &self,
        key: &str,
        now_ms: i64,
    ) -> eyre::Result<Option<CachedJsonRow>> {
        let mut rows = self
            .conn
            .query(
                "SELECT json, stale_at_ms FROM seashail_cache_json WHERE key = ?",
                (key,),
            )
            .await
            .context("query seashail_cache_json")?;

        let Some(row) = rows.next().await.context("next row")? else {
            return Ok(None);
        };

        let json: String = row.get(0).context("row.json")?;
        let stale_at_ms: i64 = row.get(1).context("row.stale_at_ms")?;

        if now_ms >= stale_at_ms {
            return Ok(None);
        }

        Ok(Some(CachedJsonRow { json }))
    }

    pub async fn upsert_json(
        &self,
        key: &str,
        json: &str,
        fetched_at_ms: i64,
        stale_at_ms: i64,
    ) -> eyre::Result<()> {
        self.conn
            .execute(
                "INSERT INTO seashail_cache_json (key, json, fetched_at_ms, stale_at_ms) \
                 VALUES (?, ?, ?, ?) \
                 ON CONFLICT(key) DO UPDATE SET \
                   json=excluded.json, \
                   fetched_at_ms=excluded.fetched_at_ms, \
                   stale_at_ms=excluded.stale_at_ms",
                (key, json, fetched_at_ms, stale_at_ms),
            )
            .await
            .context("upsert seashail_cache_json")?;
        Ok(())
    }

    pub async fn upsert_health_snapshot(&self, h: &HealthSnapshotInput<'_>) -> eyre::Result<()> {
        self.conn
            .execute(
                "INSERT INTO seashail_health_snapshots \
                   (surface, chain, provider, wallet, account_index, fetched_at_ms, payload_json) \
                 VALUES (?, ?, ?, ?, ?, ?, ?) \
                 ON CONFLICT(surface, chain, provider, wallet, account_index) DO UPDATE SET \
                   fetched_at_ms=excluded.fetched_at_ms, \
                   payload_json=excluded.payload_json",
                (
                    h.surface,
                    h.chain,
                    h.provider,
                    h.wallet,
                    h.account_index,
                    h.fetched_at_ms,
                    h.payload_json,
                ),
            )
            .await
            .context("upsert seashail_health_snapshots")?;
        Ok(())
    }

    pub async fn list_health_snapshots(&self) -> eyre::Result<Vec<HealthSnapshotRow>> {
        let mut rows = self
            .conn
            .query(
                "SELECT surface, chain, provider, wallet, account_index, fetched_at_ms, payload_json \
                 FROM seashail_health_snapshots",
                (),
            )
            .await
            .context("query seashail_health_snapshots")?;

        let mut out: Vec<HealthSnapshotRow> = vec![];
        while let Some(row) = rows.next().await.context("next row")? {
            out.push(HealthSnapshotRow {
                surface: row.get(0).context("row.surface")?,
                chain: row.get(1).context("row.chain")?,
                provider: row.get(2).context("row.provider")?,
                wallet: row.get(3).context("row.wallet")?,
                account_index: row.get(4).context("row.account_index")?,
                fetched_at_ms: row.get(5).context("row.fetched_at_ms")?,
                payload_json: row.get(6).context("row.payload_json")?,
            });
        }
        Ok(out)
    }

    pub async fn insert_portfolio_snapshot(
        &self,
        fetched_at_ms: i64,
        day: &str,
        scope_json: &str,
    ) -> eyre::Result<i64> {
        let mut rows = self
            .conn
            .query(
                "INSERT INTO seashail_portfolio_snapshots (fetched_at_ms, day, scope_json) \
                 VALUES (?, ?, ?) RETURNING snapshot_id",
                (fetched_at_ms, day, scope_json),
            )
            .await
            .context("insert seashail_portfolio_snapshots")?;
        let Some(row) = rows.next().await.context("next row")? else {
            eyre::bail!("insert portfolio snapshot returned no snapshot_id");
        };
        let id: i64 = row.get(0).context("row.snapshot_id")?;
        Ok(id)
    }

    pub async fn insert_portfolio_snapshot_item(
        &self,
        snapshot_id: i64,
        wallet: &str,
        account_index: i64,
        chain: &str,
        usd_value: f64,
        payload_json: &str,
    ) -> eyre::Result<()> {
        self.conn
            .execute(
                "INSERT INTO seashail_portfolio_snapshot_items \
                 (snapshot_id, wallet, account_index, chain, usd_value, payload_json) \
                 VALUES (?, ?, ?, ?, ?, ?)",
                (
                    snapshot_id,
                    wallet,
                    account_index,
                    chain,
                    usd_value,
                    payload_json,
                ),
            )
            .await
            .context("insert seashail_portfolio_snapshot_items")?;
        Ok(())
    }

    pub async fn list_portfolio_snapshot_totals_for_scope(
        &self,
        scope_json: &str,
        limit: usize,
    ) -> eyre::Result<Vec<PortfolioSnapshotTotalRow>> {
        let limit_i64 = i64::try_from(limit).unwrap_or(50);
        let mut rows = self
            .conn
            .query(
                "SELECT s.snapshot_id, s.fetched_at_ms, s.day, \
                        COALESCE(SUM(i.usd_value), 0) AS total_usd \
                   FROM seashail_portfolio_snapshots s \
                   LEFT JOIN seashail_portfolio_snapshot_items i \
                     ON i.snapshot_id = s.snapshot_id \
                  WHERE s.scope_json = ? \
                  GROUP BY s.snapshot_id, s.fetched_at_ms, s.day \
                  ORDER BY s.snapshot_id DESC \
                  LIMIT ?",
                (scope_json, limit_i64),
            )
            .await
            .context("query seashail_portfolio_snapshots totals")?;

        let mut out: Vec<PortfolioSnapshotTotalRow> = vec![];
        while let Some(row) = rows.next().await.context("next row")? {
            out.push(PortfolioSnapshotTotalRow {
                snapshot_id: row.get(0).context("row.snapshot_id")?,
                fetched_at_ms: row.get(1).context("row.fetched_at_ms")?,
                day: row.get(2).context("row.day")?,
                total_usd: row.get(3).context("row.total_usd")?,
            });
        }
        Ok(out)
    }

    pub async fn portfolio_snapshot_total_at_or_before(
        &self,
        scope_json: &str,
        target_ms: i64,
    ) -> eyre::Result<Option<PortfolioSnapshotTotalRow>> {
        let mut rows = self
            .conn
            .query(
                "SELECT s.snapshot_id, s.fetched_at_ms, s.day, \
                        COALESCE(SUM(i.usd_value), 0) AS total_usd \
                   FROM seashail_portfolio_snapshots s \
                   LEFT JOIN seashail_portfolio_snapshot_items i \
                     ON i.snapshot_id = s.snapshot_id \
                  WHERE s.scope_json = ? AND s.fetched_at_ms <= ? \
                  GROUP BY s.snapshot_id, s.fetched_at_ms, s.day \
                  ORDER BY s.fetched_at_ms DESC \
                  LIMIT 1",
                (scope_json, target_ms),
            )
            .await
            .context("query portfolio snapshot total at_or_before")?;

        let Some(row) = rows.next().await.context("next row")? else {
            return Ok(None);
        };
        Ok(Some(PortfolioSnapshotTotalRow {
            snapshot_id: row.get(0).context("row.snapshot_id")?,
            fetched_at_ms: row.get(1).context("row.fetched_at_ms")?,
            day: row.get(2).context("row.day")?,
            total_usd: row.get(3).context("row.total_usd")?,
        }))
    }

    pub fn now_ms() -> eyre::Result<i64> {
        let d = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .context("system clock before unix epoch")?;
        i64::try_from(d.as_millis()).context("millis since unix epoch overflowed i64")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eyre::ContextCompat as _;

    #[tokio::test]
    async fn price_cache_respects_stale_at() -> eyre::Result<()> {
        let td = tempfile::tempdir().context("create tempdir")?;
        let paths = SeashailPaths {
            config_dir: td.path().join("cfg"),
            data_dir: td.path().join("data"),
            log_file: td.path().join("data").join("seashail.log.jsonl"),
        };
        paths.ensure_private_dirs().context("ensure private dirs")?;

        let db = Db::open(&paths, true).await.context("open db")?;

        let now = 1_000_000_i64;
        db.upsert_price("k", 1.23, "Binance", now, now + 50)
            .await
            .context("upsert price")?;
        assert!(db
            .get_price_if_fresh("k", now)
            .await
            .context("get fresh price")?
            .is_some());
        assert!(db
            .get_price_if_fresh("k", now + 50)
            .await
            .context("get stale price")?
            .is_none());
        Ok(())
    }

    #[tokio::test]
    async fn portfolio_snapshot_round_trips_totals() -> eyre::Result<()> {
        let td = tempfile::tempdir().context("create tempdir")?;
        let paths = SeashailPaths {
            config_dir: td.path().join("cfg"),
            data_dir: td.path().join("data"),
            log_file: td.path().join("data").join("seashail.log.jsonl"),
        };
        paths.ensure_private_dirs().context("ensure private dirs")?;

        let db = Db::open(&paths, true).await.context("open db")?;
        let snapshot_id = db
            .insert_portfolio_snapshot(
                123,
                "2026-02-10",
                "{\"wallets\":null,\"chains\":[\"solana\"]}",
            )
            .await
            .context("insert snapshot")?;
        db.insert_portfolio_snapshot_item(
            snapshot_id,
            "w1",
            0,
            "solana",
            10.0,
            "{\"usd_value\":10}",
        )
        .await
        .context("insert item")?;
        db.insert_portfolio_snapshot_item(
            snapshot_id,
            "w1",
            0,
            "ethereum",
            5.0,
            "{\"usd_value\":5}",
        )
        .await
        .context("insert item2")?;

        let rows = db
            .list_portfolio_snapshot_totals_for_scope(
                "{\"wallets\":null,\"chains\":[\"solana\"]}",
                10,
            )
            .await
            .context("list totals")?;
        assert_eq!(rows.len(), 1);
        let first = rows.first().context("expected at least one row")?;
        assert_eq!(first.snapshot_id, snapshot_id);
        let delta = crate::financial_math::abs_f64(crate::financial_math::sum_f64(&[
            first.total_usd,
            -15.0_f64,
        ]));
        assert!(delta < 1e-9_f64);
        Ok(())
    }

    #[tokio::test]
    async fn health_snapshot_upserts_latest() -> eyre::Result<()> {
        let td = tempfile::tempdir().context("create tempdir")?;
        let paths = SeashailPaths {
            config_dir: td.path().join("cfg"),
            data_dir: td.path().join("data"),
            log_file: td.path().join("data").join("seashail.log.jsonl"),
        };
        paths.ensure_private_dirs().context("ensure private dirs")?;
        let db = Db::open(&paths, true).await.context("open db")?;

        db.upsert_health_snapshot(&HealthSnapshotInput {
            surface: "perps",
            chain: "hyperliquid",
            provider: "hyperliquid",
            wallet: "main",
            account_index: 0,
            fetched_at_ms: 100,
            payload_json: "{\"positions\":[]}",
        })
        .await?;
        db.upsert_health_snapshot(&HealthSnapshotInput {
            surface: "perps",
            chain: "hyperliquid",
            provider: "hyperliquid",
            wallet: "main",
            account_index: 0,
            fetched_at_ms: 200,
            payload_json: "{\"positions\":[1]}",
        })
        .await?;

        let rows = db.list_health_snapshots().await?;
        assert_eq!(rows.len(), 1);
        let first = rows.first().context("expected at least one row")?;
        assert_eq!(first.fetched_at_ms, 200);
        assert!(first.payload_json.contains("\"positions\""));
        Ok(())
    }

    #[tokio::test]
    async fn portfolio_snapshot_total_window_helpers_pick_expected_rows() -> eyre::Result<()> {
        let td = tempfile::tempdir().context("create tempdir")?;
        let paths = SeashailPaths {
            config_dir: td.path().join("cfg"),
            data_dir: td.path().join("data"),
            log_file: td.path().join("data").join("seashail.log.jsonl"),
        };
        paths.ensure_private_dirs().context("ensure private dirs")?;
        let db = Db::open(&paths, true).await.context("open db")?;

        let scope = r#"{"wallets":["w"],"chains":["solana"]}"#;
        let t0 = 1_000_000_i64;
        let t1 = 1_100_000_i64;
        let t2 = 1_200_000_i64;

        let s0 = db
            .insert_portfolio_snapshot(t0, "1970-01-01", scope)
            .await?;
        db.insert_portfolio_snapshot_item(s0, "w", 0, "solana", 10.0, "{}")
            .await?;

        let s1 = db
            .insert_portfolio_snapshot(t1, "1970-01-01", scope)
            .await?;
        db.insert_portfolio_snapshot_item(s1, "w", 0, "solana", 25.0, "{}")
            .await?;

        let s2 = db
            .insert_portfolio_snapshot(t2, "1970-01-01", scope)
            .await?;
        db.insert_portfolio_snapshot_item(s2, "w", 0, "solana", 40.0, "{}")
            .await?;

        let before = db
            .portfolio_snapshot_total_at_or_before(scope, t1)
            .await?
            .ok_or_else(|| eyre::eyre!("missing at_or_before"))?;
        assert_eq!(before.snapshot_id, s1);
        let expected_total_usd = 25.0_f64;
        assert!((before.total_usd - expected_total_usd).abs() < 1e-9_f64);

        Ok(())
    }
}
