use crate::sqlite::StringError;
use common::fmt::WriteJoin;
use rusqlite::{Error as SqlError, Result as SqlResult};
use std::fmt;

pub enum SqlConstraint {
    Unique(UniqueConstraint),
}

impl fmt::Display for SqlConstraint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SqlConstraint::Unique(unique) => write!(f, "{}", unique),
        }
    }
}

/// This type is used to define a UNIQUE constraint on multiple column.
/// https://www.w3schools.com/sql/sql_unique.asp
#[derive(Debug)]
pub struct UniqueConstraint {
    name: Option<&'static str>,
    columns: Vec<&'static str>,
}

impl UniqueConstraint {
    pub fn new<I>(columns: I) -> SqlResult<UniqueConstraint>
    where
        I: IntoIterator<Item = &'static str>,
    {
        let columns: Vec<_> = columns.into_iter().collect();
        if columns.is_empty() {
            let error = "SQL CONSTRAINT UNIQUE must contain columns";
            return Err(SqlError::ToSqlConversionFailure(StringError::from(error).into_boxed()));
        }
        Ok(UniqueConstraint { name: None, columns })
    }

    pub fn name(mut self, name: &'static str) -> UniqueConstraint {
        self.name = Some(name);
        self
    }
}

impl From<UniqueConstraint> for SqlConstraint {
    fn from(unique: UniqueConstraint) -> Self { SqlConstraint::Unique(unique) }
}

impl fmt::Display for UniqueConstraint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        assert!(
            !self.columns.is_empty(),
            "'UniqueColumnsConstraint::new' must check if the given columns are empty"
        );
        write!(f, "CONSTRAINT ")?;
        if let Some(name) = self.name {
            write!(f, "{} ", name)?;
        }
        write!(f, "UNIQUE (")?;

        self.columns.iter().write_join(f, ", ")?;
        write!(f, ")")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unique_constraint() {
        let constraint = UniqueConstraint::new(vec!["id", "type"])
            .unwrap()
            .name("id_type_constraint");
        let actual = constraint.to_string();
        let expected = "CONSTRAINT id_type_constraint UNIQUE (id, type)";
        assert_eq!(actual, expected);

        let constraint = UniqueConstraint::new(vec!["id"]).unwrap();
        let actual = constraint.to_string();
        let expected = "CONSTRAINT UNIQUE (id)";
        assert_eq!(actual, expected);

        UniqueConstraint::new(std::iter::empty())
            .expect_err("Expected an error on creating SQL UNIQUE CONSTRAINT without columns");
    }
}
