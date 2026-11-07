//! Runtime configuration from environment variables.

use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Config {
    /// Where the SQLite database and download scratch files live.
    pub data_dir: PathBuf,
    /// Built frontend to serve (`web/dist`).
    pub static_dir: PathBuf,
    pub port: u16,
    /// Hours between price refreshes.
    pub price_refresh_hours: u64,
    pub mtgjson_base_url: String,
}

impl Config {
    pub fn from_env() -> Self {
        let data_dir = std::env::var("PACKRAT_DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("data"));
        let static_dir = std::env::var("PACKRAT_STATIC_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("web/dist"));
        let port = std::env::var("PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(4100);
        let price_refresh_hours = std::env::var("PACKRAT_PRICE_REFRESH_HOURS")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(12);
        let mtgjson_base_url = std::env::var("PACKRAT_MTGJSON_URL")
            .unwrap_or_else(|_| "https://mtgjson.com/api/v5".to_owned());
        Self {
            data_dir,
            static_dir,
            port,
            price_refresh_hours,
            mtgjson_base_url,
        }
    }

    pub fn database_path(&self) -> PathBuf {
        self.data_dir.join("packrat.db")
    }
}
