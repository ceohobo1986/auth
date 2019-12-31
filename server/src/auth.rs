use crate::cache::TimedCache;
use argon2::Error as HashError;
use auth_common::AuthToken;
use enum_display_derive::Display;
use lazy_static::lazy_static;
use rusqlite::{params, Connection, Error as DbError, NO_PARAMS};
use serde_json::Error as JsonError;
use std::error::Error;
use std::fmt::Display;
use uuid::Uuid;

lazy_static! {
    static ref TOKENS: TimedCache<AuthToken, Uuid> = TimedCache::new();
}

fn db() -> Result<Connection, AuthError> {
    Ok(Connection::open("/opt/veloren-auth/data/auth.db")?)
}

fn salt() -> [u8; 16] {
    rand::random::<u128>().to_le_bytes()
}

#[derive(Debug, Display)]
pub enum AuthError {
    UserExists,
    UserDoesNotExist,
    InvalidLogin,
    InvalidToken,
    Db(DbError),
    Hash(HashError),
    Json(JsonError),
}

impl Error for AuthError {}

impl From<DbError> for AuthError {
    fn from(err: DbError) -> Self {
        Self::Db(err)
    }
}

impl From<HashError> for AuthError {
    fn from(err: HashError) -> Self {
        Self::Hash(err)
    }
}

impl From<JsonError> for AuthError {
    fn from(err: JsonError) -> Self {
        Self::Json(err)
    }
}

pub fn init_db() -> Result<(), AuthError> {
    db()?.execute(
        "
        CREATE TABLE IF NOT EXISTS users (
            uuid TEXT PRIMARY KEY,
            username TEXT NOT NULL UNIQUE,
            pwhash TEXT NOT NULL
        )
    ",
        NO_PARAMS,
    )?;
    Ok(())
}

fn user_exists(username: &str) -> Result<bool, AuthError> {
    let db = db()?;
    let mut stmt = db.prepare("SELECT uuid, username, pwhash FROM users WHERE username == ?1")?;
    Ok(stmt
        .query_map(params![username], |_| Ok(()))
        .unwrap()
        .count()
        == 1)
}

pub fn username_to_uuid(username: &str) -> Result<Uuid, AuthError> {
    let db = db()?;
    let mut stmt = db.prepare("SELECT uuid, username, pwhash FROM users WHERE username == ?1")?;
    let result = stmt
        .query_map(params![username], |row| row.get::<_, String>(0))?
        .filter_map(|s| s.ok())
        .filter_map(|s| Uuid::parse_str(&s).ok())
        .next()
        .ok_or(AuthError::UserDoesNotExist);
    result
}

pub fn uuid_to_username(uuid: &Uuid) -> Result<String, AuthError> {
    let db = db()?;
    let uuid = uuid.to_simple().to_string();
    let mut stmt = db.prepare("SELECT uuid, username, pwhash FROM users WHERE uuid == ?1")?;
    let result = stmt
        .query_map(params![uuid], |row| row.get::<_, String>(1))?
        .filter_map(|s| s.ok())
        .next()
        .ok_or(AuthError::UserDoesNotExist);
    result
}

pub fn register(username: &str, password: &str) -> Result<(), AuthError> {
    if user_exists(username)? {
        return Err(AuthError::UserExists);
    }
    let uuid = Uuid::new_v4().to_simple().to_string();
    let hconfig = argon2::Config::default();
    let pwhash = argon2::hash_encoded(password.as_bytes(), &salt(), &hconfig)?;
    db()?.execute(
        "INSERT INTO users (uuid, username, pwhash) VALUES(?1, ?2, ?2)",
        params![uuid, username, pwhash],
    )?;
    Ok(())
}

fn is_valid(username: &str, password: &str) -> Result<bool, AuthError> {
    let db = db()?;
    let mut stmt = db.prepare("SELECT uuid, username, pwhash FROM users WHERE username == ?1")?;
    let result = stmt
        .query_map(params![username], |row| row.get::<_, String>(2))?
        .filter_map(|s| s.ok())
        .filter_map(|correct| argon2::verify_encoded(&correct, password.as_bytes()).ok())
        .next()
        .ok_or(AuthError::InvalidLogin);
    result
}

pub fn generate_token(username: &str, password: &str) -> Result<AuthToken, AuthError> {
    if !is_valid(username, password)? {
        return Err(AuthError::InvalidLogin);
    }

    let uuid = username_to_uuid(username)?;
    let token = AuthToken::generate();
    TOKENS.insert(token, uuid);
    Ok(token)
}

pub fn verify(token: AuthToken) -> Result<Uuid, AuthError> {
    let mut uuid = None;
    TOKENS.run(&token, |maybe_entry| {
        if let Some(entry) = maybe_entry {
            uuid = Some(entry.data.clone());
            false
        } else {
            false
        }
    });
    uuid.ok_or(AuthError::InvalidToken)
}
