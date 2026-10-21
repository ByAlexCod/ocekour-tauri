//! Pilotes en LECTURE SEULE : PostgreSQL (tokio-postgres), SQL Server (tiberius), Oracle
//! (oracle-rs, pur Rust). Chaque appel ouvre une connexion, exécute dans une transaction en
//! lecture seule quand le moteur le permet, puis ferme.

use crate::config::DatabaseConnection;
use serde::Serialize;
use serde_json::{Value, json};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct QueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
    pub row_count: usize,
    pub truncated: bool,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct TableInfo {
    pub schema: String,
    pub name: String,
    /// `table` ou `view`.
    pub kind: &'static str,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ColumnInfo {
    pub name: String,
    pub r#type: String,
    pub nullable: bool,
}

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

fn hex_cell(bytes: &[u8]) -> Value {
    let hex = hex::encode(bytes);
    json!(format!("0x{}", &hex[..hex.len().min(2_000)]))
}

fn finish(columns: Vec<String>, mut rows: Vec<Vec<Value>>, max: usize, started: Instant) -> QueryResult {
    let row_count = rows.len();
    let truncated = rows.len() > max;
    rows.truncate(max);
    QueryResult { columns, rows, row_count, truncated, elapsed_ms: started.elapsed().as_millis() as u64 }
}

async fn timed<T>(timeout: Duration, what: &str, f: impl std::future::Future<Output = anyhow::Result<T>>) -> anyhow::Result<T> {
    tokio::time::timeout(timeout, f).await.map_err(|_| anyhow::anyhow!("{what} : délai de {} s dépassé.", timeout.as_secs()))?
}

// --- PostgreSQL ---------------------------------------------------------------------------------

mod pg {
    use super::*;
    use tokio_postgres::types::Type;

    /// Message du serveur (« relation "x" does not exist ») plutôt que « db error ».
    pub fn e(err: tokio_postgres::Error) -> anyhow::Error {
        match err.as_db_error() {
            Some(db) => anyhow::anyhow!("{}{}", db.message(), db.detail().map(|d| format!(" ({d})")).unwrap_or_default()),
            None => {
                let source = std::error::Error::source(&err).map(|s| format!(" : {s}")).unwrap_or_default();
                anyhow::anyhow!("{err}{source}")
            }
        }
    }

    pub async fn connect(c: &DatabaseConnection, password: &str, timeout: Duration) -> anyhow::Result<tokio_postgres::Client> {
        let mut config = tokio_postgres::Config::new();
        config
            .host(&c.host)
            .port(c.port_or_default())
            .user(&c.user)
            .password(password)
            .application_name("Ocekour")
            .connect_timeout(CONNECT_TIMEOUT)
            .options(&format!("-c statement_timeout={}", timeout.as_millis()));
        if let Some(db) = c.database.as_deref().filter(|d| !d.is_empty()) { config.dbname(db); }
        let client = if c.ssl {
            config.ssl_mode(tokio_postgres::config::SslMode::Require);
            let tls = tokio_postgres_rustls::MakeRustlsConnect::new(crate::net::rustls_config(&c.tls)?);
            let (client, conn) = timed(CONNECT_TIMEOUT, "Connexion", async { Ok(config.connect(tls).await.map_err(e)?) }).await?;
            tokio::spawn(conn);
            client
        } else {
            let (client, conn) = timed(CONNECT_TIMEOUT, "Connexion", async { Ok(config.connect(tokio_postgres::NoTls).await.map_err(e)?) }).await?;
            tokio::spawn(conn);
            client
        };
        Ok(client)
    }

    /// Valeur texte (protocole simple) → JSON selon le type de la colonne.
    pub fn cell(text: Option<&str>, ty: Option<&Type>) -> Value {
        let Some(t) = text else { return Value::Null };
        match ty {
            Some(&Type::BOOL) => json!(t == "t"),
            Some(&Type::INT2) | Some(&Type::INT4) | Some(&Type::OID) => t.parse::<i64>().map(|n| json!(n)).unwrap_or(json!(t)),
            Some(&Type::FLOAT4) | Some(&Type::FLOAT8) => t.parse::<f64>().ok().and_then(|f| serde_json::Number::from_f64(f).map(Value::Number)).unwrap_or(json!(t)),
            Some(&Type::TIMESTAMPTZ) => chrono::DateTime::parse_from_str(&format!("{t}00"), "%Y-%m-%d %H:%M:%S%.f%z")
                .or_else(|_| chrono::DateTime::parse_from_str(t, "%Y-%m-%d %H:%M:%S%.f%:z"))
                .map(|d| json!(crate::text::iso(d.with_timezone(&chrono::Utc))))
                .unwrap_or(json!(t)),
            Some(&Type::TIMESTAMP) => chrono::NaiveDateTime::parse_from_str(t, "%Y-%m-%d %H:%M:%S%.f")
                .map(|d| json!(d.format("%Y-%m-%dT%H:%M:%S%.3f").to_string()))
                .unwrap_or(json!(t)),
            Some(&Type::BYTEA) => match t.strip_prefix("\\x").and_then(|h| hex::decode(h).ok()) {
                Some(b) => hex_cell(&b),
                None => json!(t),
            },
            _ => json!(t),
        }
    }

    pub async fn query(c: &DatabaseConnection, password: &str, sql: &str, max: usize, timeout: Duration) -> anyhow::Result<QueryResult> {
        let started = Instant::now();
        let client = connect(c, password, timeout).await?;
        timed(timeout, "Requête", async {
            // Types des colonnes (sans exécuter), puis exécution en protocole simple (texte).
            let types: Vec<Type> = client.prepare(sql).await.map(|s| s.columns().iter().map(|c| c.type_().clone()).collect()).unwrap_or_default();
            client.batch_execute("BEGIN TRANSACTION READ ONLY").await.map_err(e)?;
            let messages = client.simple_query(sql).await;
            let _ = client.batch_execute("ROLLBACK").await;
            let mut columns = vec![];
            let mut rows = vec![];
            for m in messages.map_err(e)? {
                match m {
                    tokio_postgres::SimpleQueryMessage::RowDescription(cols) => columns = cols.iter().map(|c| c.name().to_string()).collect(),
                    tokio_postgres::SimpleQueryMessage::Row(row) => {
                        if columns.is_empty() { columns = row.columns().iter().map(|c| c.name().to_string()).collect(); }
                        if rows.len() <= max {
                            rows.push((0..row.len()).map(|i| cell(row.get(i), types.get(i))).collect());
                        } else {
                            rows.push(vec![]);
                        }
                    }
                    _ => {}
                }
            }
            let count = rows.len();
            let mut r = finish(columns, rows, max, started);
            r.row_count = count;
            Ok(r)
        })
        .await
    }

    pub async fn test(c: &DatabaseConnection, password: &str) -> anyhow::Result<String> {
        let client = connect(c, password, Duration::from_secs(30)).await?;
        Ok(client.query_one("SELECT version()", &[]).await.map_err(e)?.get(0))
    }

    pub async fn tables(c: &DatabaseConnection, password: &str) -> anyhow::Result<Vec<TableInfo>> {
        let client = connect(c, password, Duration::from_secs(30)).await?;
        let rows = client
            .query("SELECT table_schema::text, table_name::text, table_type::text FROM information_schema.tables WHERE table_schema NOT IN ('pg_catalog','information_schema') ORDER BY 1,2", &[])
            .await
            .map_err(e)?;
        Ok(rows.iter().map(|r| TableInfo { schema: r.get(0), name: r.get(1), kind: if r.get::<_, String>(2).to_uppercase().contains("VIEW") { "view" } else { "table" } }).collect())
    }

    pub async fn columns(c: &DatabaseConnection, password: &str, table: &str, schema: Option<&str>) -> anyhow::Result<Vec<ColumnInfo>> {
        let client = connect(c, password, Duration::from_secs(30)).await?;
        let rows = client
            .query(
                "SELECT column_name::text, data_type::text, is_nullable::text FROM information_schema.columns WHERE table_name = $1 AND ($2::text IS NULL OR table_schema = $2) ORDER BY ordinal_position",
                &[&table, &schema],
            )
            .await
            .map_err(e)?;
        Ok(rows.iter().map(|r| ColumnInfo { name: r.get(0), r#type: r.get(1), nullable: r.get::<_, String>(2) == "YES" }).collect())
    }
}

// --- SQL Server ---------------------------------------------------------------------------------

mod mssql {
    use super::*;
    use tiberius::{AuthMethod, Client, ColumnData, Config, EncryptionLevel, FromSql};
    use tokio::net::TcpStream;
    use tokio_util::compat::{Compat, TokioAsyncWriteCompatExt};

    type Conn = Client<Compat<TcpStream>>;

    pub async fn connect(c: &DatabaseConnection, password: &str) -> anyhow::Result<Conn> {
        let mut config = Config::new();
        config.host(&c.host);
        config.port(c.port_or_default());
        if let Some(db) = c.database.as_deref().filter(|d| !d.is_empty()) { config.database(db); }
        config.authentication(AuthMethod::sql_server(&c.user, password));
        config.application_name("Ocekour");
        config.readonly(true);
        if c.ssl {
            config.encryption(EncryptionLevel::Required);
            if c.tls.insecure { config.trust_cert(); } else if let Some(ca) = &c.tls.ca_file { config.trust_cert_ca(ca); }
        } else {
            config.encryption(EncryptionLevel::NotSupported);
        }
        timed(CONNECT_TIMEOUT, "Connexion", async {
            let tcp = TcpStream::connect(config.get_addr()).await?;
            tcp.set_nodelay(true)?;
            Ok(Client::connect(config, tcp.compat_write()).await?)
        })
        .await
    }

    pub fn cell(d: &ColumnData<'static>) -> Value {
        let date = |d: &ColumnData<'static>| -> Option<Value> {
            if let Ok(Some(v)) = chrono::DateTime::<chrono::Utc>::from_sql(d) { return Some(json!(crate::text::iso(v))); }
            if let Ok(Some(v)) = chrono::NaiveDateTime::from_sql(d) { return Some(json!(v.format("%Y-%m-%dT%H:%M:%S%.3f").to_string())); }
            if let Ok(Some(v)) = chrono::NaiveDate::from_sql(d) { return Some(json!(v.to_string())); }
            if let Ok(Some(v)) = chrono::NaiveTime::from_sql(d) { return Some(json!(v.to_string())); }
            None
        };
        match d {
            ColumnData::U8(v) => json!(v),
            ColumnData::I16(v) => json!(v),
            ColumnData::I32(v) => json!(v),
            ColumnData::I64(v) => v.map(|n| json!(n.to_string())).unwrap_or(Value::Null),
            ColumnData::F32(v) => json!(v),
            ColumnData::F64(v) => json!(v),
            ColumnData::Bit(v) => json!(v),
            ColumnData::String(v) => json!(v.as_deref()),
            ColumnData::Guid(v) => json!(v.map(|g| g.to_string().to_uppercase())),
            ColumnData::Binary(v) => v.as_deref().map(hex_cell).unwrap_or(Value::Null),
            ColumnData::Numeric(v) => v.map(|n| { let s = n.to_string(); s.parse::<f64>().ok().and_then(serde_json::Number::from_f64).map(Value::Number).unwrap_or(json!(s)) }).unwrap_or(Value::Null),
            ColumnData::Xml(v) => json!(v.as_ref().map(|x| x.to_string())),
            other => date(other).unwrap_or(Value::Null),
        }
    }

    pub async fn query(c: &DatabaseConnection, password: &str, sql: &str, max: usize, timeout: Duration) -> anyhow::Result<QueryResult> {
        let started = Instant::now();
        let mut client = connect(c, password).await?;
        timed(timeout, "Requête", async {
            let mut stream = client.simple_query(sql).await?;
            let columns: Vec<String> = stream.columns().await?.map(|cols| cols.iter().map(|c| c.name().to_string()).collect()).unwrap_or_default();
            let first = stream.into_first_result().await?;
            let count = first.len();
            let rows = first.into_iter().take(max + 1).map(|r| r.into_iter().map(|d| cell(&d)).collect()).collect();
            let mut r = finish(columns, rows, max, started);
            r.row_count = count;
            Ok(r)
        })
        .await
    }

    async fn strings(client: &mut Conn, sql: &str, params: &[&dyn tiberius::ToSql]) -> anyhow::Result<Vec<Vec<Option<String>>>> {
        let rows = client.query(sql, params).await?.into_first_result().await?;
        Ok(rows.into_iter().map(|r| r.into_iter().map(|d| match cell(&d) { Value::Null => None, Value::String(s) => Some(s), v => Some(v.to_string()) }).collect()).collect())
    }

    pub async fn test(c: &DatabaseConnection, password: &str) -> anyhow::Result<String> {
        let mut client = connect(c, password).await?;
        let rows = strings(&mut client, "SELECT @@VERSION AS v", &[]).await?;
        Ok(rows.first().and_then(|r| r[0].clone()).unwrap_or_default().lines().next().unwrap_or_default().to_string())
    }

    pub async fn tables(c: &DatabaseConnection, password: &str) -> anyhow::Result<Vec<TableInfo>> {
        let mut client = connect(c, password).await?;
        let rows = strings(&mut client, "SELECT TABLE_SCHEMA, TABLE_NAME, TABLE_TYPE FROM INFORMATION_SCHEMA.TABLES ORDER BY 1,2", &[]).await?;
        Ok(rows.into_iter().map(|r| TableInfo {
            schema: r[0].clone().unwrap_or_default(),
            name: r[1].clone().unwrap_or_default(),
            kind: if r[2].as_deref().unwrap_or_default().to_uppercase().contains("VIEW") { "view" } else { "table" },
        }).collect())
    }

    pub async fn columns(c: &DatabaseConnection, password: &str, table: &str, schema: Option<&str>) -> anyhow::Result<Vec<ColumnInfo>> {
        let mut client = connect(c, password).await?;
        let rows = strings(
            &mut client,
            "SELECT COLUMN_NAME, DATA_TYPE, IS_NULLABLE FROM INFORMATION_SCHEMA.COLUMNS WHERE TABLE_NAME = @P1 AND (@P2 IS NULL OR TABLE_SCHEMA = @P2) ORDER BY ORDINAL_POSITION",
            &[&table, &schema],
        )
        .await?;
        Ok(rows.into_iter().map(|r| ColumnInfo { name: r[0].clone().unwrap_or_default(), r#type: r[1].clone().unwrap_or_default(), nullable: r[2].as_deref() == Some("YES") }).collect())
    }
}

// --- Oracle -------------------------------------------------------------------------------------

mod oracle {
    use super::*;
    use oracle_rs::{Config, Connection, Value as O};

    pub async fn connect(c: &DatabaseConnection, password: &str) -> anyhow::Result<Connection> {
        let service = c.database.clone().unwrap_or_default();
        let mut config = if c.oracle_sid {
            Config::with_sid(&c.host, c.port_or_default(), service, &c.user, password)
        } else {
            Config::new(&c.host, c.port_or_default(), service, &c.user, password)
        };
        config = config.connect_timeout(CONNECT_TIMEOUT);
        if c.ssl {
            let tls = oracle_rs::TlsConfig {
                verify_server: !c.tls.insecure,
                ca_cert_path: c.tls.ca_file.clone().filter(|_| !c.tls.insecure),
                ssl_server_dn_match: !c.tls.insecure,
                ..Default::default()
            };
            config = config.tls_config(tls);
        }
        timed(CONNECT_TIMEOUT, "Connexion", async { Ok(Connection::connect_with_config(config).await?) }).await
    }

    pub async fn cell(conn: &Connection, v: &O) -> Value {
        match v {
            O::Null => Value::Null,
            O::String(s) => json!(s),
            O::Integer(n) => json!(n),
            O::Float(f) => serde_json::Number::from_f64(*f).map(Value::Number).unwrap_or(Value::Null),
            O::Boolean(b) => json!(b),
            O::Bytes(b) => hex_cell(b),
            O::Json(j) => json!(j.to_string()),
            O::Lob(oracle_rs::types::LobValue::Inline(b)) => json!(String::from_utf8_lossy(b)),
            O::Lob(oracle_rs::types::LobValue::Locator(l)) => match conn.read_clob(l).await {
                Ok(s) => json!(s),
                Err(_) => Value::Null,
            },
            O::Lob(_) => Value::Null,
            other => json!(other.to_string()),
        }
    }

    async fn all_rows(conn: &Connection, sql: &str, params: &[O], max: usize) -> anyhow::Result<(Vec<String>, Vec<Vec<Value>>)> {
        let mut result = conn.query(sql, params).await?;
        let columns: Vec<String> = result.columns.iter().map(|c| c.name.clone()).collect();
        let mut raw = std::mem::take(&mut result.rows);
        while result.has_more_rows && raw.len() <= max {
            result = conn.fetch_more(result.cursor_id, &result.columns, 100).await?;
            raw.extend(std::mem::take(&mut result.rows));
        }
        let mut rows = vec![];
        for row in raw.iter().take(max + 1) {
            let mut cells = vec![];
            for i in 0..columns.len() { cells.push(cell(conn, row.get(i).unwrap_or(&O::Null)).await); }
            rows.push(cells);
        }
        Ok((columns, rows))
    }

    pub async fn query(c: &DatabaseConnection, password: &str, sql: &str, max: usize, timeout: Duration) -> anyhow::Result<QueryResult> {
        let started = Instant::now();
        let conn = connect(c, password).await?;
        let result = timed(timeout, "Requête", async {
            let _ = conn.execute("SET TRANSACTION READ ONLY", &[]).await;
            let (columns, rows) = all_rows(&conn, sql, &[], max).await?;
            Ok(finish(columns, rows, max, started))
        })
        .await;
        let _ = conn.rollback().await;
        let _ = conn.close().await;
        result
    }

    pub async fn test(c: &DatabaseConnection, password: &str) -> anyhow::Result<String> {
        let conn = connect(c, password).await?;
        let (_, rows) = all_rows(&conn, "SELECT banner FROM v$version WHERE ROWNUM = 1", &[], 1).await?;
        let _ = conn.close().await;
        Ok(rows.first().and_then(|r| r.first()).and_then(|v| v.as_str().map(str::to_string)).unwrap_or_else(|| "Oracle".into()))
    }

    const SYSTEM: &str = "('SYS','SYSTEM','XDB','MDSYS','CTXSYS','DBSNMP','OUTLN','ORDSYS','WMSYS','APEX_040000','OLAPSYS','GSMADMIN_INTERNAL','AUDSYS','LBACSYS','DVSYS','ORDDATA')";

    pub async fn tables(c: &DatabaseConnection, password: &str) -> anyhow::Result<Vec<TableInfo>> {
        let conn = connect(c, password).await?;
        let sql = format!("SELECT owner, table_name, 'table' kind FROM all_tables WHERE owner NOT IN {SYSTEM} UNION ALL SELECT owner, view_name, 'view' FROM all_views WHERE owner NOT IN {SYSTEM} ORDER BY 1, 2");
        let (_, rows) = all_rows(&conn, &sql, &[], 100_000).await?;
        let _ = conn.close().await;
        let s = |v: &Value| v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string());
        Ok(rows.iter().map(|r| TableInfo { schema: s(&r[0]), name: s(&r[1]), kind: if r[2] == "view" { "view" } else { "table" } }).collect())
    }

    pub async fn columns(c: &DatabaseConnection, password: &str, table: &str, schema: Option<&str>) -> anyhow::Result<Vec<ColumnInfo>> {
        let conn = connect(c, password).await?;
        let owner = schema.map(|s| O::String(s.to_uppercase())).unwrap_or(O::Null);
        let (_, rows) = all_rows(
            &conn,
            "SELECT column_name, data_type, nullable FROM all_tab_columns WHERE table_name = :1 AND (:2 IS NULL OR owner = :3) ORDER BY column_id",
            &[O::String(table.to_uppercase()), owner.clone(), owner],
            100_000,
        )
        .await?;
        let _ = conn.close().await;
        let s = |v: &Value| v.as_str().map(str::to_string).unwrap_or_default();
        Ok(rows.iter().map(|r| ColumnInfo { name: s(&r[0]), r#type: s(&r[1]), nullable: r[2] == "Y" }).collect())
    }
}

/// Requête de lecture déjà validée.
pub async fn query(c: &DatabaseConnection, password: &str, sql: &str, max: usize, timeout: Duration) -> anyhow::Result<QueryResult> {
    match c.engine.as_str() {
        "postgres" => pg::query(c, password, sql, max, timeout).await,
        "sqlserver" => mssql::query(c, password, sql, max, timeout).await,
        _ => oracle::query(c, password, sql, max, timeout).await,
    }
}

/// Vérifie la connexion ; renvoie la version du serveur.
pub async fn test(c: &DatabaseConnection, password: &str) -> anyhow::Result<String> {
    match c.engine.as_str() {
        "postgres" => pg::test(c, password).await,
        "sqlserver" => mssql::test(c, password).await,
        _ => oracle::test(c, password).await,
    }
}

pub async fn tables(c: &DatabaseConnection, password: &str) -> anyhow::Result<Vec<TableInfo>> {
    match c.engine.as_str() {
        "postgres" => pg::tables(c, password).await,
        "sqlserver" => mssql::tables(c, password).await,
        _ => oracle::tables(c, password).await,
    }
}

pub async fn columns(c: &DatabaseConnection, password: &str, table: &str, schema: Option<&str>) -> anyhow::Result<Vec<ColumnInfo>> {
    match c.engine.as_str() {
        "postgres" => pg::columns(c, password, table, schema).await,
        "sqlserver" => mssql::columns(c, password, table, schema).await,
        _ => oracle::columns(c, password, table, schema).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_postgres::types::Type;

    #[test]
    fn cellules_postgres() {
        assert_eq!(pg::cell(Some("t"), Some(&Type::BOOL)), json!(true));
        assert_eq!(pg::cell(Some("42"), Some(&Type::INT4)), json!(42));
        assert_eq!(pg::cell(Some("9007199254740993"), Some(&Type::INT8)), json!("9007199254740993"));
        assert_eq!(pg::cell(Some("1.5"), Some(&Type::FLOAT8)), json!(1.5));
        assert_eq!(pg::cell(Some("2026-09-25 08:00:00+00"), Some(&Type::TIMESTAMPTZ)), json!("2026-09-25T08:00:00.000Z"));
        assert_eq!(pg::cell(Some("2026-09-25 10:00:00.5+02"), Some(&Type::TIMESTAMPTZ)), json!("2026-09-25T08:00:00.500Z"));
        assert_eq!(pg::cell(Some("\\x0aff"), Some(&Type::BYTEA)), json!("0x0aff"));
        assert_eq!(pg::cell(None, Some(&Type::TEXT)), Value::Null);
    }
}
