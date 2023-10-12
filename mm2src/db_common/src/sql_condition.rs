use rusqlite::Result as SqlResult;
use sql_builder::SqlBuilder;

use crate::sql_value::{FromQuoted, SqlValue, SqlValueOptional, SqlValueToString, SqlValuesToStrings};
use crate::sqlite::{OwnedSqlParam, SqlParamsBuilder, ToValidSqlIdent};

/// An SQL condition builder.
pub trait SqlCondition: Sized {
    fn sql_builder(&mut self) -> &mut SqlBuilder;

    fn sql_params(&mut self) -> &mut SqlParamsBuilder;

    /// Add WHERE condition for equal parts.
    /// For more details see [`SqlBuilder::and_where_eq`] and [`SqlBuilder::and_where_is_null`].
    ///
    /// Please note the function validates the given `field` name,
    /// and `value` is considered a valid as it's able to be converted into `SqlValue`.
    fn and_where_eq<S, T>(&mut self, field: S, value: T) -> SqlResult<&mut Self>
    where
        S: ToValidSqlIdent,
        SqlValueOptional: From<T>,
    {
        let field = field.to_valid_sql_ident()?;
        match SqlValueOptional::from(value) {
            SqlValueOptional::Some(value) => self.sql_builder().and_where_eq(field, SqlValue::value_to_string(value)),
            SqlValueOptional::Null => self.sql_builder().and_where_is_null(field),
        };
        Ok(self)
    }

    /// Add WHERE condition for equal parts.
    /// For more details see [`SqlBuilder::and_where_eq`].
    ///
    /// Please note the function validates the given `field`.
    fn and_where_eq_param<S, T>(&mut self, field: S, param: T) -> SqlResult<&mut Self>
    where
        S: ToValidSqlIdent,
        OwnedSqlParam: From<T>,
    {
        let field = field.to_valid_sql_ident()?;
        match OwnedSqlParam::from(param) {
            OwnedSqlParam::Null => self.sql_builder().and_where_is_null(field),
            not_null => {
                let param_id = self.sql_params().push_owned_param(not_null);
                self.sql_builder().and_where_eq(field, param_id)
            },
        };
        Ok(self)
    }

    /// Add WHERE field IN (list).
    /// For more details see [`SqlBuilder::and_where_in`].
    ///
    /// Please note the function validates the given `field`,
    /// and `values` are considered valid as they're able to be converted into `SqlValue`.
    #[inline]
    fn and_where_in<S, I, T>(&mut self, field: S, values: I) -> SqlResult<&mut Self>
    where
        S: ToValidSqlIdent,
        I: IntoIterator<Item = T>,
        SqlValue: From<T>,
    {
        self.sql_builder()
            .and_where_in(field.to_valid_sql_ident()?, &SqlValue::values_to_strings(values));
        Ok(self)
    }

    /// Add WHERE field IN (string list).
    /// For more details see [`SqlBuilder::and_where_in_quoted`].
    ///
    /// Please note the function validates the given `field`,
    /// and `values` are considered valid as they're able to be converted into `SqlValue`.
    #[inline]
    fn and_where_in_quoted<S, I, T>(&mut self, field: S, values: I) -> SqlResult<&mut Self>
    where
        S: ToValidSqlIdent,
        I: IntoIterator<Item = T>,
        SqlValue: FromQuoted<T>,
    {
        let values = SqlValue::quoted_values_to_strings(values);
        self.sql_builder().and_where_in(field.to_valid_sql_ident()?, &values);
        Ok(self)
    }

    /// Add WHERE field IN (string list) with the specified `params`.
    /// For more details see [`SqlBuilder::and_where_in`].
    ///
    /// Please note the function validates the given `field`.
    #[inline]
    fn and_where_in_params<S, I, P>(&mut self, field: S, params: I) -> SqlResult<&mut Self>
    where
        S: ToValidSqlIdent,
        I: IntoIterator<Item = P>,
        OwnedSqlParam: From<P>,
    {
        let param_ids = self.sql_params().push_params(params);
        self.sql_builder().and_where_in(field.to_valid_sql_ident()?, &param_ids);
        Ok(self)
    }

    /// Add OR condition of equal parts to the last WHERE condition.
    /// For more details see [`SqlBuilder::or_where_eq`].
    ///
    /// Please note the function validates the given `field`,
    /// and `value` is considered a valid as it's able to be converted into `SqlValue`.
    fn or_where_eq<S, T>(&mut self, field: S, value: T) -> SqlResult<&mut Self>
    where
        S: ToValidSqlIdent,
        SqlValueOptional: From<T>,
    {
        let field = field.to_valid_sql_ident()?;
        match SqlValueOptional::from(value) {
            SqlValueOptional::Some(value) => self.sql_builder().or_where_eq(field, SqlValue::value_to_string(value)),
            SqlValueOptional::Null => self.sql_builder().or_where_is_null(field),
        };
        Ok(self)
    }

    /// Add OR condition of equal parts to the last WHERE condition.
    /// For more details see [`SqlBuilder::or_where_eq`].
    ///
    /// Please note the function validates the given `field`.
    fn or_where_eq_param<S, T>(&mut self, field: S, param: T) -> SqlResult<&mut Self>
    where
        S: ToValidSqlIdent,
        OwnedSqlParam: From<T>,
    {
        let field = field.to_valid_sql_ident()?;
        match OwnedSqlParam::from(param) {
            OwnedSqlParam::Null => self.sql_builder().or_where_is_null(field),
            not_null => {
                let param_id = self.sql_params().push_owned_param(not_null);
                self.sql_builder().or_where_eq(field, param_id)
            },
        };
        Ok(self)
    }

    /// Add OR field IN (list) to the last WHERE condition.
    /// For more details see [`SqlBuilder::or_where_in`].
    ///
    /// Please note the function validates the given `field`,
    /// and `values` are considered valid as they're able to be converted into `SqlValue`.
    #[inline]
    fn or_where_in<S, I, T>(&mut self, field: S, values: I) -> SqlResult<&mut Self>
    where
        S: ToValidSqlIdent,
        I: IntoIterator<Item = T>,
        SqlValue: From<T>,
    {
        self.sql_builder()
            .or_where_in(field.to_valid_sql_ident()?, &SqlValue::values_to_strings(values));
        Ok(self)
    }

    /// Add OR field IN (string list) to the last WHERE condition.
    /// For more details see [`SqlBuilder::and_where_in`].
    ///
    /// Please note the function validates the given `field`,
    /// and `values` are considered valid as they're able to be converted into `SqlValue`.
    #[inline]
    fn or_where_in_quoted<S, I, T>(&mut self, field: S, values: I) -> SqlResult<&mut Self>
    where
        S: ToValidSqlIdent,
        I: IntoIterator<Item = T>,
        SqlValue: FromQuoted<T>,
    {
        let values: Vec<_> = values.into_iter().map(SqlValue::quoted_value_to_string).collect();
        self.sql_builder().or_where_in(field.to_valid_sql_ident()?, &values);
        Ok(self)
    }

    /// Add OR field IN (list) to the last WHERE condition.
    /// For more details see [`SqlBuilder::or_where_in`].
    ///
    /// Please note the function validates the given `field`.
    #[inline]
    fn or_where_in_params<S, I, P>(&mut self, field: S, params: I) -> SqlResult<&mut Self>
    where
        S: ToValidSqlIdent,
        I: IntoIterator<Item = P>,
        OwnedSqlParam: From<P>,
    {
        let param_ids = self.sql_params().push_params(params);
        self.sql_builder().or_where_in(field.to_valid_sql_ident()?, &param_ids);
        Ok(self)
    }
}
