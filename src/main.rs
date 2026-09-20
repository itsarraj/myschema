use clap::Parser;
use myschema::diff::{diff_schemas, generate_sql, Column, Schema, Table};
use sqlx::mysql::MySqlPoolOptions;
use sqlx::Row;

#[derive(Parser)]
#[command(
    name = "myschema",
    about = "Diffs two live MySQL schemas and generates reconciling SQL"
)]
struct Cli {
    /// Connection URL of the "before" (source) database.
    from: String,
    /// Connection URL of the "after" (target) database.
    to: String,
}

async fn load_schema(url: &str) -> anyhow::Result<Schema> {
    let pool = MySqlPoolOptions::new().connect(url).await?;
    // information_schema's own text columns come back as VARBINARY on
    // this server (a real, live-caught MySQL/sqlx mismatch — sqlx's
    // typed `String` decode rejects VARBINARY outright), so every
    // column is explicitly cast to CHAR to force a real VARCHAR result.
    let rows = sqlx::query(
        "SELECT CAST(TABLE_NAME AS CHAR) AS table_name, \
                CAST(COLUMN_NAME AS CHAR) AS column_name, \
                CAST(COLUMN_TYPE AS CHAR) AS column_type, \
                CAST(IS_NULLABLE AS CHAR) AS is_nullable \
         FROM information_schema.columns \
         WHERE table_schema = DATABASE() \
         ORDER BY TABLE_NAME, ORDINAL_POSITION",
    )
    .fetch_all(&pool)
    .await?;

    let mut schema: Schema = Schema::new();
    for row in rows {
        let table_name: String = row.get("table_name");
        let column_name: String = row.get("column_name");
        let column_type: String = row.get("column_type");
        let is_nullable: String = row.get("is_nullable");
        let table: &mut Table = schema.entry(table_name).or_default();
        table.insert(
            column_name.clone(),
            Column {
                name: column_name,
                data_type: column_type,
                nullable: is_nullable == "YES",
            },
        );
    }
    Ok(schema)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let from = load_schema(&cli.from).await?;
    let to = load_schema(&cli.to).await?;

    let changes = diff_schemas(&from, &to);
    if changes.is_empty() {
        println!("-- schemas are identical, nothing to do");
        return Ok(());
    }

    println!("-- {} change(s) found:", changes.len());
    for statement in generate_sql(&changes, &to) {
        println!("{statement}");
    }
    Ok(())
}
