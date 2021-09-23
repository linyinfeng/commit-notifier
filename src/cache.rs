use rusqlite::{params, Connection};

use crate::error::Error;

pub fn initialize(cache: &Connection) -> Result<(), Error> {
    cache.execute(
        "CREATE TABLE commits_cache (
               target_commit TEXT    NOT NULL,
               this_commit   TEXT    NOT NULL,
               is_parent     INTEGER NOT NULL
             )",
        [],
    )?;
    cache.execute(
        "CREATE UNIQUE INDEX idx_commit_pair
             ON commits_cache (target_commit, this_commit)",
        [],
    )?;

    Ok(())
}

pub fn query(cache: &Connection, target: &str, commit: &str) -> Result<Option<bool>, Error> {
    let mut stmt = cache.prepare_cached(
        "SELECT is_parent FROM commits_cache WHERE target_commit = ?1 AND this_commit = ?2",
    )?;
    log::trace!("query cache: ({}, {})", target, commit);
    let query_result: Vec<bool> = stmt
        .query_map(params!(target, commit), |row| row.get(0))?
        .collect::<Result<_, _>>()?;
    match query_result.len() {
        0 => Ok(None),
        1 => Ok(Some(query_result[0])),
        _ => panic!("internal cache format error"),
    }
}

pub fn store(cache: &Connection, target: &str, commit: &str, hit: bool) -> Result<(), Error> {
    let mut stmt = cache.prepare_cached(
        "INSERT INTO commits_cache (target_commit, this_commit, is_parent) VALUES (?1, ?2, ?3)",
    )?;
    log::trace!("insert new cache: ({}, {}, {})", target, commit, hit);
    let inserted = stmt.execute(params!(target, commit, hit))?;
    assert_eq!(inserted, 1);
    Ok(())
}

pub fn remove(cache: &Connection, target: &str) -> Result<(), Error> {
    let mut stmt = cache.prepare_cached(
        "DELETE FROM commits_cache WHERE target_commit = ?1",
    )?;
    log::trace!("delete cache for target commit: {}", target);
    stmt.execute(params!(target))?;
    Ok(())
}
