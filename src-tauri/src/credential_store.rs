use tauri::AppHandle;

pub enum GetError {
    NoEntry,
    Other(String),
}

pub fn set_password(app: &AppHandle, service: &str, account: &str, password: &str) -> Result<(), String> {
    #[cfg(not(target_os = "android"))]
    {
        let _ = app;
        let entry = keyring::Entry::new(service, account)
            .map_err(|e| format!("无法访问系统凭据存储：{e}"))?;
        entry
            .set_password(password)
            .map_err(|e| format!("无法保存凭据：{e}"))
    }
    #[cfg(target_os = "android")]
    {
        android_store::set(app, service, account, password)
    }
}

pub fn get_password(app: &AppHandle, service: &str, account: &str) -> Result<String, GetError> {
    #[cfg(not(target_os = "android"))]
    {
        let _ = app;
        let entry = keyring::Entry::new(service, account)
            .map_err(|e| GetError::Other(format!("无法访问系统凭据存储：{e}")))?;
        match entry.get_password() {
            Ok(v) => Ok(v),
            Err(keyring::Error::NoEntry) => Err(GetError::NoEntry),
            Err(e) => Err(GetError::Other(e.to_string())),
        }
    }
    #[cfg(target_os = "android")]
    {
        android_store::get(app, service, account)
    }
}

#[cfg(target_os = "android")]
mod android_store {
    use super::*;
    use rusqlite::{params, Connection, OptionalExtension};
    use tauri::Manager;

    fn db(app: &AppHandle) -> Result<Connection, String> {
        let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let conn = Connection::open(dir.join("credentials.sqlite3")).map_err(|e| e.to_string())?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
             CREATE TABLE IF NOT EXISTS credentials (service TEXT NOT NULL, account TEXT NOT NULL, secret TEXT NOT NULL, PRIMARY KEY (service, account));",
        )
        .map_err(|e| e.to_string())?;
        Ok(conn)
    }

    pub fn set(app: &AppHandle, service: &str, account: &str, password: &str) -> Result<(), String> {
        db(app)?
            .execute(
                "INSERT INTO credentials(service, account, secret) VALUES (?1, ?2, ?3) ON CONFLICT(service, account) DO UPDATE SET secret=excluded.secret",
                params![service, account, password],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn get(app: &AppHandle, service: &str, account: &str) -> Result<String, GetError> {
        let conn = db(app).map_err(GetError::Other)?;
        let secret: Option<String> = conn
            .query_row(
                "SELECT secret FROM credentials WHERE service=?1 AND account=?2",
                params![service, account],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| GetError::Other(e.to_string()))?;
        secret.ok_or(GetError::NoEntry)
    }
}
