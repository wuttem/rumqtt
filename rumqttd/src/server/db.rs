use sqlx::{error::DatabaseError, AnyPool, Row};
use std::sync::Arc;
use tracing::{error, info};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum DbError {
    #[error("Database error: {0}")]
    Sqlx(#[from] sqlx::Error),
}

#[derive(Clone)]
pub struct Database {
    pool: AnyPool,
}

impl Database {
    pub async fn new(db_url: &str) -> Result<Self, DbError> {
        sqlx::any::install_default_drivers();
        let pool = AnyPool::connect(db_url).await?;
        
        // Ensure the absolute table format is created
        let create_table_query = r#"
            CREATE TABLE IF NOT EXISTS users (
                username VARCHAR PRIMARY KEY,
                password VARCHAR NOT NULL
            )
        "#;
        
        sqlx::query(create_table_query).execute(&pool).await?;
        info!("Database initialized and users table checked.");

        Ok(Self { pool })
    }

    pub async fn add_user(&self, username: &str, password: &str) -> Result<(), DbError> {
        sqlx::query("INSERT INTO users (username, password) VALUES ($1, $2)")
            .bind(username)
            .bind(password)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn get_user(&self, username: &str) -> Result<Option<String>, DbError> {
        let row: Option<sqlx::any::AnyRow> = sqlx::query("SELECT password FROM users WHERE username = $1")
            .bind(username)
            .fetch_optional(&self.pool)
            .await?;

        Ok(row.map(|r| r.get("password")))
    }

    pub async fn delete_user(&self, username: &str) -> Result<(), DbError> {
        sqlx::query("DELETE FROM users WHERE username = $1")
            .bind(username)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn verify_user(&self, username: &str, password: &str) -> Result<bool, DbError> {
        let stored_password = self.get_user(username).await?;
        match stored_password {
            Some(stored) => Ok(stored == password),
            None => Ok(false),
        }
    }
}
