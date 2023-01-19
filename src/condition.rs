pub mod in_branch;

use serde::{Deserialize, Serialize};

use crate::error::Error;
use crate::repo::results::CommitResults;

use self::in_branch::InBranchCondition;

pub trait Condition {
    fn meet(&self, result: &CommitResults) -> bool;
}

#[derive(clap::ValueEnum, Serialize, Deserialize, Clone, Debug, Copy)]
pub enum Kind {
    InBranch,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GeneralCondition {
    InBranch(InBranchCondition),
}

impl GeneralCondition {
    pub fn parse(kind: Kind, expr: &str) -> Result<GeneralCondition, Error> {
        match kind {
            Kind::InBranch => Ok(GeneralCondition::InBranch(InBranchCondition::parse(expr)?)),
        }
    }
}

impl Condition for GeneralCondition {
    fn meet(&self, result: &CommitResults) -> bool {
        match self {
            GeneralCondition::InBranch(c) => c.meet(result),
        }
    }
}
