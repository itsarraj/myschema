//! Pure schema-diffing logic: given two schemas' real table/column
//! metadata, compute what's different and generate reconciling SQL.
//! No database access here — `main.rs` is the only part that queries
//! `INFORMATION_SCHEMA`.

use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Column {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
}

pub type Table = BTreeMap<String, Column>;
pub type Schema = BTreeMap<String, Table>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    TableAdded(String),
    TableRemoved(String),
    ColumnAdded {
        table: String,
        column: Column,
    },
    ColumnRemoved {
        table: String,
        column: String,
    },
    ColumnTypeChanged {
        table: String,
        column: String,
        from: String,
        to: String,
    },
    ColumnNullabilityChanged {
        table: String,
        column: String,
        data_type: String,
        now_nullable: bool,
    },
}

/// Every real difference between `from` and `to`, in a stable order:
/// table-level changes first, then column-level changes grouped by table.
pub fn diff_schemas(from: &Schema, to: &Schema) -> Vec<Change> {
    let mut changes = Vec::new();

    for table_name in to.keys() {
        if !from.contains_key(table_name) {
            changes.push(Change::TableAdded(table_name.clone()));
        }
    }
    for table_name in from.keys() {
        if !to.contains_key(table_name) {
            changes.push(Change::TableRemoved(table_name.clone()));
        }
    }

    for (table_name, to_table) in to {
        let Some(from_table) = from.get(table_name) else {
            continue;
        };
        for (col_name, to_col) in to_table {
            match from_table.get(col_name) {
                None => changes.push(Change::ColumnAdded {
                    table: table_name.clone(),
                    column: to_col.clone(),
                }),
                Some(from_col) => {
                    if from_col.data_type != to_col.data_type {
                        changes.push(Change::ColumnTypeChanged {
                            table: table_name.clone(),
                            column: col_name.clone(),
                            from: from_col.data_type.clone(),
                            to: to_col.data_type.clone(),
                        });
                    }
                    if from_col.nullable != to_col.nullable {
                        changes.push(Change::ColumnNullabilityChanged {
                            table: table_name.clone(),
                            column: col_name.clone(),
                            data_type: to_col.data_type.clone(),
                            now_nullable: to_col.nullable,
                        });
                    }
                }
            }
        }
        for col_name in from_table.keys() {
            if !to_table.contains_key(col_name) {
                changes.push(Change::ColumnRemoved {
                    table: table_name.clone(),
                    column: col_name.clone(),
                });
            }
        }
    }

    changes
}

/// Renders each change as the real MySQL DDL statement that would apply
/// it — `from`'s schema, brought forward to match `to`.
pub fn generate_sql(changes: &[Change], to: &Schema) -> Vec<String> {
    changes
        .iter()
        .map(|c| match c {
            Change::TableAdded(name) => {
                let cols = to[name]
                    .values()
                    .map(|col| {
                        format!(
                            "  {} {} {}",
                            col.name,
                            col.data_type,
                            if col.nullable { "NULL" } else { "NOT NULL" }
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(",\n");
                format!("CREATE TABLE {name} (\n{cols}\n);")
            }
            Change::TableRemoved(name) => format!("DROP TABLE {name};"),
            Change::ColumnAdded { table, column } => {
                format!(
                    "ALTER TABLE {table} ADD COLUMN {} {} {};",
                    column.name,
                    column.data_type,
                    if column.nullable { "NULL" } else { "NOT NULL" }
                )
            }
            Change::ColumnRemoved { table, column } => {
                format!("ALTER TABLE {table} DROP COLUMN {column};")
            }
            Change::ColumnTypeChanged {
                table, column, to, ..
            } => {
                format!("ALTER TABLE {table} MODIFY COLUMN {column} {to};")
            }
            Change::ColumnNullabilityChanged {
                table,
                column,
                data_type,
                now_nullable,
            } => {
                // MySQL's MODIFY COLUMN requires the full column
                // definition, not just the nullability keyword — found
                // live: the type-less form this originally generated
                // (`MODIFY COLUMN email NOT NULL`) is a real syntax
                // error against a real MySQL server, not valid partial
                // syntax the way it might look.
                format!(
                    "ALTER TABLE {table} MODIFY COLUMN {column} {data_type} {};",
                    if *now_nullable { "NULL" } else { "NOT NULL" }
                )
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn col(name: &str, ty: &str, nullable: bool) -> Column {
        Column {
            name: name.to_string(),
            data_type: ty.to_string(),
            nullable,
        }
    }

    fn schema(tables: &[(&str, &[Column])]) -> Schema {
        tables
            .iter()
            .map(|(name, cols)| {
                let table: Table = cols.iter().map(|c| (c.name.clone(), c.clone())).collect();
                (name.to_string(), table)
            })
            .collect()
    }

    #[test]
    fn detects_added_table() {
        let from = schema(&[]);
        let to = schema(&[("users", &[col("id", "int", false)])]);
        let changes = diff_schemas(&from, &to);
        assert_eq!(changes, vec![Change::TableAdded("users".to_string())]);
    }

    #[test]
    fn detects_removed_table() {
        let from = schema(&[("users", &[col("id", "int", false)])]);
        let to = schema(&[]);
        let changes = diff_schemas(&from, &to);
        assert_eq!(changes, vec![Change::TableRemoved("users".to_string())]);
    }

    #[test]
    fn detects_added_column() {
        let from = schema(&[("users", &[col("id", "int", false)])]);
        let to = schema(&[(
            "users",
            &[col("id", "int", false), col("email", "varchar(255)", true)],
        )]);
        let changes = diff_schemas(&from, &to);
        assert_eq!(
            changes,
            vec![Change::ColumnAdded {
                table: "users".to_string(),
                column: col("email", "varchar(255)", true)
            }]
        );
    }

    #[test]
    fn detects_removed_column() {
        let from = schema(&[(
            "users",
            &[col("id", "int", false), col("email", "varchar(255)", true)],
        )]);
        let to = schema(&[("users", &[col("id", "int", false)])]);
        let changes = diff_schemas(&from, &to);
        assert_eq!(
            changes,
            vec![Change::ColumnRemoved {
                table: "users".to_string(),
                column: "email".to_string()
            }]
        );
    }

    #[test]
    fn detects_column_type_change() {
        let from = schema(&[("users", &[col("age", "int", true)])]);
        let to = schema(&[("users", &[col("age", "bigint", true)])]);
        let changes = diff_schemas(&from, &to);
        assert_eq!(
            changes,
            vec![Change::ColumnTypeChanged {
                table: "users".to_string(),
                column: "age".to_string(),
                from: "int".to_string(),
                to: "bigint".to_string()
            }]
        );
    }

    #[test]
    fn detects_nullability_change() {
        let from = schema(&[("users", &[col("email", "varchar(255)", true)])]);
        let to = schema(&[("users", &[col("email", "varchar(255)", false)])]);
        let changes = diff_schemas(&from, &to);
        assert_eq!(
            changes,
            vec![Change::ColumnNullabilityChanged {
                table: "users".to_string(),
                column: "email".to_string(),
                data_type: "varchar(255)".to_string(),
                now_nullable: false
            }]
        );
    }

    #[test]
    fn identical_schemas_produce_no_changes() {
        let s = schema(&[("users", &[col("id", "int", false)])]);
        assert!(diff_schemas(&s, &s).is_empty());
    }

    #[test]
    fn generates_create_table_sql_for_added_table() {
        let to = schema(&[("users", &[col("id", "int", false)])]);
        let changes = vec![Change::TableAdded("users".to_string())];
        let sql = generate_sql(&changes, &to);
        assert!(sql[0].starts_with("CREATE TABLE users"));
        assert!(sql[0].contains("id int NOT NULL"));
    }

    #[test]
    fn generates_alter_table_add_column_sql() {
        let to = schema(&[("users", &[col("email", "varchar(255)", true)])]);
        let changes = vec![Change::ColumnAdded {
            table: "users".to_string(),
            column: col("email", "varchar(255)", true),
        }];
        let sql = generate_sql(&changes, &to);
        assert_eq!(
            sql[0],
            "ALTER TABLE users ADD COLUMN email varchar(255) NULL;"
        );
    }

    #[test]
    fn nullability_change_sql_includes_the_real_data_type() {
        // A real bug caught live: MySQL's MODIFY COLUMN requires a full
        // column definition, not just the nullability keyword — the
        // type-less form this used to generate is a genuine syntax
        // error against a real MySQL server.
        let to = schema(&[("users", &[col("email", "varchar(255)", false)])]);
        let changes = vec![Change::ColumnNullabilityChanged {
            table: "users".to_string(),
            column: "email".to_string(),
            data_type: "varchar(255)".to_string(),
            now_nullable: false,
        }];
        let sql = generate_sql(&changes, &to);
        assert_eq!(
            sql[0],
            "ALTER TABLE users MODIFY COLUMN email varchar(255) NOT NULL;"
        );
    }

    #[test]
    fn generates_drop_table_sql() {
        let to = schema(&[]);
        let changes = vec![Change::TableRemoved("users".to_string())];
        assert_eq!(generate_sql(&changes, &to), vec!["DROP TABLE users;"]);
    }
}
