use std::collections::BTreeSet;

use rusqlite::{Connection, params};

use crate::error::Error;

pub fn initialize(cache: &Connection) -> Result<(), Error> {
    cache.execute(
        "CREATE TABLE IF NOT EXISTS commits_cache (
            branch        TEXT    NOT NULL,
            commit_hash   TEXT    NOT NULL
        )",
        [],
    )?;
    cache.execute(
        "CREATE TABLE IF NOT EXISTS branches (
            branch           TEXT    NOT NULL PRIMARY KEY,
            current_commit   TEXT    NOT NULL
        )",
        [],
    )?;
    cache.execute(
        "CREATE INDEX IF NOT EXISTS idx_commit_branches
         ON commits_cache (commit_hash)",
        [],
    )?;

    Ok(())
}

pub fn branches(cache: &Connection) -> Result<BTreeSet<String>, Error> {
    let mut stmt = cache.prepare_cached("SELECT branch FROM branches;")?;
    let query_result: BTreeSet<String> = stmt
        .query_map([], |row| row.get(0))?
        .collect::<Result<_, _>>()?;
    Ok(query_result)
}

pub fn remove_branch(cache: &Connection, branch: &str) -> Result<(), Error> {
    log::trace!("delete branch \"{branch}\" from cache");
    let mut stmt1 = cache.prepare_cached("DELETE FROM branches WHERE branch = ?1")?;
    stmt1.execute(params!(branch))?;
    let mut stmt2 = cache.prepare_cached("DELETE FROM commits_cache WHERE branch = ?1")?;
    stmt2.execute(params!(branch))?;
    Ok(())
}

pub fn query_branch(cache: &Connection, branch: &str) -> Result<String, Error> {
    let mut stmt =
        cache.prepare_cached("SELECT current_commit FROM branches WHERE branch = ?1;")?;
    log::trace!("query branch: {branch}");
    let query_result: Vec<String> = stmt
        .query_map(params!(branch), |row| row.get(0))?
        .collect::<Result<_, _>>()?;
    if query_result.len() != 1 {
        Err(Error::UnknownBranch(branch.to_string()))
    } else {
        Ok(query_result[0].clone())
    }
}

pub fn store_branch(cache: &Connection, branch: &str, commit: &str) -> Result<(), Error> {
    let mut stmt =
        cache.prepare_cached("INSERT INTO branches (branch, current_commit) VALUES (?1, ?2)")?;
    log::trace!("insert new branch record: ({branch}, {commit})");
    let inserted = stmt.execute(params!(branch, commit))?;
    assert_eq!(inserted, 1);
    Ok(())
}

pub fn update_branch(cache: &Connection, branch: &str, commit: &str) -> Result<(), Error> {
    let mut stmt =
        cache.prepare_cached("UPDATE branches SET current_commit = ?2 WHERE branch = ?1")?;
    log::trace!("update branch record: ({branch}, {commit})");
    stmt.execute(params!(branch, commit))?;
    Ok(())
}

pub fn query_cache(cache: &Connection, branch: &str, commit: &str) -> Result<bool, Error> {
    let mut stmt = cache
        .prepare_cached("SELECT * FROM commits_cache WHERE branch = ?1 AND commit_hash = ?2")?;
    log::trace!("query cache: ({branch}, {commit})");
    let mut query_result = stmt.query(params!(branch, commit))?;
    if let Some(_row) = query_result.next()? {
        Ok(true)
    } else {
        Ok(false)
    }
}

pub fn query_cache_commit(cache: &Connection, commit: &str) -> Result<BTreeSet<String>, Error> {
    let mut stmt =
        cache.prepare_cached("SELECT branch FROM commits_cache WHERE commit_hash = ?1")?;
    log::trace!("query cache: {commit}");
    Ok(stmt
        .query_map(params!(commit), |row| row.get(0))?
        .collect::<Result<_, _>>()?)
}

pub fn store_cache(cache: &Connection, branch: &str, commit: &str) -> Result<(), Error> {
    let mut stmt =
        cache.prepare_cached("INSERT INTO commits_cache (branch, commit_hash) VALUES (?1, ?2)")?;
    log::trace!("insert new cache: ({branch}, {commit})");
    let inserted = stmt.execute(params!(branch, commit))?;
    assert_eq!(inserted, 1);
    Ok(())
}

pub fn batch_store_cache<I>(cache: &Connection, branch: &str, commits: I) -> Result<(), Error>
where
    I: IntoIterator<Item = String>,
{
    let mut count = 0usize;
    for c in commits.into_iter() {
        store_cache(cache, branch, &c)?;
        count += 1;
        if count.is_multiple_of(100000) {
            log::debug!("batch storing cache, current count: {count}",);
        }
    }
    Ok(())
}
