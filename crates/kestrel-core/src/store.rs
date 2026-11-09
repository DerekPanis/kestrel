//! Persistent list of paired consoles.
//!
//! Layout: `<config dir>/consoles.json` holds non-secret metadata in clear and,
//! per console, one protected blob (see `secure`) with regist key, morning and
//! the PSN account ID. The file is rewritten atomically (write temp + rename).

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde::{Deserialize, Serialize};

use crate::account::AccountId;
use crate::regist::Pairing;
use crate::secure::{self, Secret};
use crate::{Error, Result};

const SECRET_LEN: usize = 16 + 16 + 8 + 4;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ConsoleRecord {
    /// Console MAC in lowercase hex (matches discovery's host_id).
    pub id: String,
    pub nickname: String,
    /// Last known address. Discovery updates it when the console moves.
    pub host: String,
    pub ps5: bool,
    pub target: i32,
    pub rp_key_type: u32,
    pub paired_at: u64,
    pub last_connected: Option<u64>,
    pub protection: String,
    /// base64 of the protected secret blob.
    secret: String,
}

/// Decrypted credentials for one console; only lives while connecting.
pub struct Credentials {
    pub regist_key: Secret<16>,
    pub morning: Secret<16>,
    pub account: AccountId,
    pub console_pin: u32,
}

impl ConsoleRecord {
    pub fn new(host: &str, pairing: &Pairing, account: AccountId) -> Result<Self> {
        let mut plain = Vec::with_capacity(SECRET_LEN);
        plain.extend_from_slice(pairing.regist_key.expose());
        plain.extend_from_slice(pairing.morning.expose());
        plain.extend_from_slice(&account.0);
        plain.extend_from_slice(&pairing.console_pin.to_le_bytes());
        let blob = secure::protect(&plain);
        plain.fill(0);
        Ok(ConsoleRecord {
            id: pairing.mac_hex(),
            nickname: pairing.nickname.clone(),
            host: host.to_string(),
            ps5: pairing.is_ps5(),
            target: pairing.target,
            rp_key_type: pairing.rp_key_type,
            paired_at: now(),
            last_connected: None,
            protection: secure::scheme().to_string(),
            secret: STANDARD.encode(blob?),
        })
    }

    pub fn credentials(&self) -> Result<Credentials> {
        let blob = STANDARD
            .decode(&self.secret)
            .map_err(|_| Error::Store("corrupt secret".into()))?;
        let mut plain = secure::unprotect(&blob)?;
        if plain.len() != SECRET_LEN {
            plain.fill(0);
            return Err(Error::Store("secret has unexpected length".into()));
        }
        let mut regist_key = [0u8; 16];
        let mut morning = [0u8; 16];
        let mut account = [0u8; 8];
        let mut pin = [0u8; 4];
        regist_key.copy_from_slice(&plain[0..16]);
        morning.copy_from_slice(&plain[16..32]);
        account.copy_from_slice(&plain[32..40]);
        pin.copy_from_slice(&plain[40..44]);
        plain.fill(0);
        Ok(Credentials {
            regist_key: Secret::new(regist_key),
            morning: Secret::new(morning),
            account: AccountId(account),
            console_pin: u32::from_le_bytes(pin),
        })
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct StoreFile {
    version: u32,
    consoles: Vec<ConsoleRecord>,
}

pub struct ConsoleStore {
    path: PathBuf,
    file: StoreFile,
}

impl ConsoleStore {
    /// Opens `%APPDATA%\Kestrel\Kestrel\config\consoles.json` (or the platform equivalent).
    pub fn open_default() -> Result<Self> {
        let dirs = directories::ProjectDirs::from("app", "Kestrel", "Kestrel")
            .ok_or_else(|| Error::Store("no home directory".into()))?;
        Self::open(dirs.config_dir().join("consoles.json"))
    }

    pub fn open(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        let file = match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|e| Error::Store(format!("{}: {e}", path.display())))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => StoreFile {
                version: 1,
                consoles: vec![],
            },
            Err(e) => return Err(Error::Store(e.to_string())),
        };
        Ok(ConsoleStore { path, file })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn consoles(&self) -> &[ConsoleRecord] {
        &self.file.consoles
    }

    /// Finds by id, nickname (case-insensitive) or host address.
    pub fn find(&self, key: &str) -> Option<&ConsoleRecord> {
        let k = key.trim();
        self.file
            .consoles
            .iter()
            .find(|c| c.id == k.to_lowercase() || c.nickname.eq_ignore_ascii_case(k) || c.host == k)
    }

    /// Inserts or replaces (re-pairing the same console replaces its record).
    pub fn upsert(&mut self, rec: ConsoleRecord) -> Result<()> {
        self.file.consoles.retain(|c| c.id != rec.id);
        self.file.consoles.push(rec);
        self.save()
    }

    pub fn remove(&mut self, id: &str) -> Result<bool> {
        let before = self.file.consoles.len();
        self.file.consoles.retain(|c| c.id != id);
        let removed = self.file.consoles.len() != before;
        if removed {
            self.save()?;
        }
        Ok(removed)
    }

    pub fn update_host(&mut self, id: &str, host: &str) -> Result<()> {
        if let Some(c) = self.file.consoles.iter_mut().find(|c| c.id == id) {
            if c.host != host {
                c.host = host.to_string();
                self.save()?;
            }
        }
        Ok(())
    }

    pub fn mark_connected(&mut self, id: &str) -> Result<()> {
        if let Some(c) = self.file.consoles.iter_mut().find(|c| c.id == id) {
            c.last_connected = Some(now());
            self.save()?;
        }
        Ok(())
    }

    fn save(&self) -> Result<()> {
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir).map_err(|e| Error::Store(e.to_string()))?;
        }
        let json =
            serde_json::to_vec_pretty(&self.file).map_err(|e| Error::Store(e.to_string()))?;
        let tmp = self.path.with_extension("json.tmp");
        write_private(&tmp, &json)?;
        fs::rename(&tmp, &self.path).map_err(|e| Error::Store(e.to_string()))
    }
}

#[cfg(unix)]
fn write_private(path: &Path, data: &[u8]) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| Error::Store(e.to_string()))?;
    f.write_all(data).map_err(|e| Error::Store(e.to_string()))
}

#[cfg(not(unix))]
fn write_private(path: &Path, data: &[u8]) -> Result<()> {
    fs::write(path, data).map_err(|e| Error::Store(e.to_string()))
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairing() -> Pairing {
        Pairing {
            target: 1_000_100,
            nickname: "PS5-123".into(),
            mac: [0xaa, 0xbb, 0xcc, 0x01, 0x02, 0x03],
            regist_key: Secret::new(*b"1a2b3c4d\0\0\0\0\0\0\0\0"),
            morning: Secret::new([7u8; 16]),
            rp_key_type: 1,
            console_pin: 1234,
        }
    }

    #[test]
    fn persist_and_reload() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("consoles.json");
        let account = AccountId([1, 2, 3, 4, 5, 6, 7, 8]);
        {
            let mut store = ConsoleStore::open(&path).unwrap();
            store
                .upsert(ConsoleRecord::new("192.168.1.50", &pairing(), account).unwrap())
                .unwrap();
        }
        let store = ConsoleStore::open(&path).unwrap();
        let rec = store.find("ps5-123").expect("found by nickname");
        assert_eq!(rec.id, "aabbcc010203");
        assert!(rec.ps5);
        assert!(store.find("192.168.1.50").is_some());
        let creds = rec.credentials().unwrap();
        assert_eq!(creds.regist_key.expose(), b"1a2b3c4d\0\0\0\0\0\0\0\0");
        assert_eq!(creds.morning.expose(), &[7u8; 16]);
        assert_eq!(creds.account, account);
        assert_eq!(creds.console_pin, 1234);
    }

    #[test]
    fn upsert_replaces_and_remove_works() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = ConsoleStore::open(dir.path().join("c.json")).unwrap();
        let account = AccountId([0; 8]);
        store
            .upsert(ConsoleRecord::new("10.0.0.2", &pairing(), account).unwrap())
            .unwrap();
        store
            .upsert(ConsoleRecord::new("10.0.0.3", &pairing(), account).unwrap())
            .unwrap();
        assert_eq!(store.consoles().len(), 1);
        assert_eq!(store.consoles()[0].host, "10.0.0.3");
        assert!(store.remove("aabbcc010203").unwrap());
        assert!(store.consoles().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn file_is_owner_only_and_secret_not_in_clear_fields() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("c.json");
        let mut store = ConsoleStore::open(&path).unwrap();
        store
            .upsert(ConsoleRecord::new("10.0.0.2", &pairing(), AccountId([0; 8])).unwrap())
            .unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let text = fs::read_to_string(&path).unwrap();
        assert!(
            !text.contains("1a2b3c4d"),
            "regist key must not appear as plain text"
        );
    }
}
