pub mod in_branch;
pub mod suppress_from_to;

use serde::{Deserialize, Serialize};

use crate::{
    chat::results::CommitCheckResult, condition::suppress_from_to::SuppressFromToCondition,
    error::Error,
};

use self::in_branch::InBranchCondition;

pub trait Condition {
    fn check(&self, check_results: &CommitCheckResult) -> Action;
}

#[derive(clap::ValueEnum, Serialize, Deserialize, Clone, Debug, Copy)]
pub enum Kind {
    RemoveIfInBranch,
    SuppressFromTo,
}

#[derive(
    clap::ValueEnum, Serialize, Deserialize, Clone, Debug, Copy, PartialEq, Eq, PartialOrd, Ord,
)]
pub enum Action {
    None,
    Remove,
    SuppressNotification,
}

impl Action {
    pub fn is_none(self) -> bool {
        matches!(self, Action::None)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GeneralCondition {
    InBranch(InBranchCondition),
    SuppressFromTo(SuppressFromToCondition),
}

impl GeneralCondition {
    pub fn parse(kind: Kind, expr: &str) -> Result<GeneralCondition, Error> {
        match kind {
            Kind::RemoveIfInBranch => {
                Ok(GeneralCondition::InBranch(InBranchCondition::parse(expr)?))
            }
            Kind::SuppressFromTo => Ok(GeneralCondition::SuppressFromTo(
                SuppressFromToCondition::parse(expr)?,
            )),
        }
    }
}

impl Condition for GeneralCondition {
    fn check(&self, check_results: &CommitCheckResult) -> Action {
        match self {
            GeneralCondition::InBranch(c) => c.check(check_results),
            GeneralCondition::SuppressFromTo(c) => c.check(check_results),
        }
    }
}
