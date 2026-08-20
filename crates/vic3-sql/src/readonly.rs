//! Reject DDL/DML before DataFusion plans the statement.

use datafusion::sql::sqlparser::ast::Statement;
use datafusion::sql::sqlparser::dialect::GenericDialect;
use datafusion::sql::sqlparser::parser::Parser;

use crate::SqlError;

/// Ensure every top-level statement is a read-only `SELECT` / `WITH…SELECT` /
/// `EXPLAIN` of those.
pub fn assert_readonly(sql: &str) -> Result<(), SqlError> {
    let dialect = GenericDialect {};
    let statements = Parser::parse_sql(&dialect, sql)
        .map_err(|e| SqlError::read_only(format!("parse failed: {e}")))?;
    if statements.is_empty() {
        return Err(SqlError::read_only("empty SQL"));
    }
    for stmt in statements {
        check_statement(&stmt)?;
    }
    Ok(())
}

fn check_statement(stmt: &Statement) -> Result<(), SqlError> {
    match stmt {
        Statement::Query(_) => Ok(()),
        Statement::Explain { statement, .. } => check_statement(statement),
        other => Err(SqlError::read_only(format!(
            "only SELECT / WITH…SELECT / EXPLAIN SELECT are allowed (got {other})"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_select_and_with() {
        assert_readonly("SELECT 1").unwrap();
        assert_readonly("WITH x AS (SELECT 1) SELECT * FROM x").unwrap();
        assert_readonly("EXPLAIN SELECT 1").unwrap();
    }

    #[test]
    fn rejects_ddl_dml() {
        for sql in [
            "CREATE TABLE t (a INT)",
            "INSERT INTO t VALUES (1)",
            "UPDATE t SET a = 1",
            "DELETE FROM t",
            "DROP TABLE t",
            "ATTACH 'x' AS db",
        ] {
            let err = assert_readonly(sql).expect_err(sql);
            assert!(matches!(err, SqlError::ReadOnly(_)), "{sql}: {err}");
        }
    }
}
