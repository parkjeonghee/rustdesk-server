use async_trait::async_trait;
use hbb_common::{log, ResultType};
use sqlx::{
    sqlite::SqliteConnectOptions, ConnectOptions, Connection, Error as SqlxError, Row,
    SqliteConnection,
};
use std::{ops::DerefMut, str::FromStr};
//use sqlx::postgres::PgPoolOptions;
//use sqlx::mysql::MySqlPoolOptions;

type Pool = deadpool::managed::Pool<DbPool>;

pub struct DbPool {
    url: String,
}

#[async_trait]
impl deadpool::managed::Manager for DbPool {
    type Type = SqliteConnection;
    type Error = SqlxError;
    async fn create(&self) -> Result<SqliteConnection, SqlxError> {
        let mut opt = SqliteConnectOptions::from_str(&self.url).unwrap();
        opt.log_statements(log::LevelFilter::Debug);
        SqliteConnection::connect_with(&opt).await
    }
    async fn recycle(
        &self,
        obj: &mut SqliteConnection,
    ) -> deadpool::managed::RecycleResult<SqlxError> {
        Ok(obj.ping().await?)
    }
}

#[derive(Clone)]
pub struct Database {
    pool: Pool,
}

#[derive(Default)]
pub struct Peer {
    pub guid: Vec<u8>,
    pub id: String,
    pub uuid: Vec<u8>,
    pub pk: Vec<u8>,
    pub user: Option<Vec<u8>>,
    pub info: String,
    pub status: Option<i64>,
}

impl Database {
    pub async fn new(url: &str) -> ResultType<Database> {
        if !std::path::Path::new(url).exists() {
            std::fs::File::create(url).ok();
        }
        let n: usize = std::env::var("MAX_DATABASE_CONNECTIONS")
            .unwrap_or_else(|_| "1".to_owned())
            .parse()
            .unwrap_or(1);
        log::debug!("MAX_DATABASE_CONNECTIONS={}", n);
        let pool = Pool::new(
            DbPool {
                url: url.to_owned(),
            },
            n,
        );
        let _ = pool.get().await?; // test
        let db = Database { pool };
        db.create_tables().await?;
        Ok(db)
    }

    async fn create_tables(&self) -> ResultType<()> {
        sqlx::query(
            "create table if not exists peer (
                guid blob primary key not null,
                id varchar(100) not null,
                uuid blob not null,
                pk blob not null,
                created_at datetime not null default(current_timestamp),
                user blob,
                status tinyint,
                note varchar(300),
                info text not null
            ) without rowid;"
        )
        .execute(self.pool.get().await?.deref_mut())
        .await?;
        sqlx::query("create unique index if not exists index_peer_id on peer (id)")
            .execute(self.pool.get().await?.deref_mut())
            .await?;
        sqlx::query("create index if not exists index_peer_user on peer (user)")
            .execute(self.pool.get().await?.deref_mut())
            .await?;
        sqlx::query("create index if not exists index_peer_created_at on peer (created_at)")
            .execute(self.pool.get().await?.deref_mut())
            .await?;
        sqlx::query("create index if not exists index_peer_status on peer (status)")
            .execute(self.pool.get().await?.deref_mut())
            .await?;
        Ok(())
    }

    pub async fn get_peer(&self, id: &str) -> ResultType<Option<Peer>> {
        let row = sqlx::query(
            "select guid, id, uuid, pk, user, status, info from peer where id = ?"
        )
        .bind(id)
        .fetch_optional(self.pool.get().await?.deref_mut())
        .await?;
        Ok(row.map(|r| Peer {
            guid: r.get("guid"),
            id: r.get("id"),
            uuid: r.get("uuid"),
            pk: r.get("pk"),
            user: r.get("user"),
            info: r.get("info"),
            status: r.get("status"),
        }))
    }

    pub async fn insert_peer(
        &self,
        id: &str,
        uuid: &[u8],
        pk: &[u8],
        info: &str,
    ) -> ResultType<Vec<u8>> {
        let guid = uuid::Uuid::new_v4().as_bytes().to_vec();
        sqlx::query("insert into peer(guid, id, uuid, pk, info) values(?, ?, ?, ?, ?)")
            .bind(&guid)
            .bind(id)
            .bind(uuid)
            .bind(pk)
            .bind(info)
            .execute(self.pool.get().await?.deref_mut())
            .await?;
        Ok(guid)
    }

    pub async fn update_pk(
        &self,
        guid: &Vec<u8>,
        id: &str,
        pk: &[u8],
        info: &str,
    ) -> ResultType<()> {
        sqlx::query("update peer set id=?, pk=?, info=? where guid=?")
            .bind(id)
            .bind(pk)
            .bind(info)
            .bind(guid)
            .execute(self.pool.get().await?.deref_mut())
            .await?;
        Ok(())
    }

    pub async fn get_peers(&self, limit: i64, offset: i64) -> ResultType<Vec<Peer>> {
        let rows = sqlx::query(
            "select guid, id, uuid, pk, user, status, info from peer order by created_at desc limit ? offset ?"
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(self.pool.get().await?.deref_mut())
        .await?;
        Ok(rows.into_iter().map(|r| Peer {
            guid: r.get("guid"),
            id: r.get("id"),
            uuid: r.get("uuid"),
            pk: r.get("pk"),
            user: r.get("user"),
            info: r.get("info"),
            status: r.get("status"),
        }).collect())
    }

    pub async fn update_peer_status(&self, id: &str, status: i64) -> ResultType<()> {
        sqlx::query("update peer set status=? where id=?")
            .bind(status)
            .bind(id)
            .execute(self.pool.get().await?.deref_mut())
            .await?;
        Ok(())
    }

    pub async fn set_all_peer_status_offline(&self) -> ResultType<u64> {
        let result = sqlx::query("update peer set status=0 where status=1")
            .execute(self.pool.get().await?.deref_mut())
            .await?;
        Ok(result.rows_affected())
    }

    pub async fn delete_peer(&self, id: &str) -> ResultType<u64> {
        let result = sqlx::query("delete from peer where id = ?")
            .bind(id)
            .execute(self.pool.get().await?.deref_mut())
            .await?;
        Ok(result.rows_affected())
    }
}

#[cfg(test)]
mod tests {
    use hbb_common::tokio;
    #[test]
    fn test_insert() {
        insert();
    }

    #[tokio::main(flavor = "multi_thread")]
    async fn insert() {
        let db = super::Database::new("test.sqlite3").await.unwrap();
        let mut jobs = vec![];
        for i in 0..10000 {
            let cloned = db.clone();
            let id = i.to_string();
            let a = tokio::spawn(async move {
                let empty_vec = Vec::new();
                cloned
                    .insert_peer(&id, &empty_vec, &empty_vec, "")
                    .await
                    .unwrap();
            });
            jobs.push(a);
        }
        for i in 0..10000 {
            let cloned = db.clone();
            let id = i.to_string();
            let a = tokio::spawn(async move {
                cloned.get_peer(&id).await.unwrap();
            });
            jobs.push(a);
        }
        hbb_common::futures::future::join_all(jobs).await;
    }
}
