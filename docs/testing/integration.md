# Integration Testing

## Philosophy

Integration tests exercise **real components** (Postgres, Redis, Tantivy, MinIO) in containers.
They verify contracts between services, not internal logic.

## Testcontainers setup

```rust
// tests/support/containers.rs
use testcontainers::{core::WaitFor, runners::AsyncRunner, ContainerAsync, GenericImage};

pub async fn postgres() -> ContainerAsync<GenericImage> {
    GenericImage::new("postgres", "16-alpine")
        .with_env_var("POSTGRES_USER", "lynx")
        .with_env_var("POSTGRES_PASSWORD", "lynx")
        .with_env_var("POSTGRES_DB", "lynx_test")
        .with_exposed_port(5432)
        .with_wait_for(WaitFor::message_on_stdout("database system is ready to accept connections"))
        .start()
        .await
        .unwrap()
}

pub async fn redis() -> ContainerAsync<GenericImage> {
    GenericImage::new("redis", "7-alpine")
        .with_exposed_port(6379)
        .with_wait_for(WaitFor::message_on_stdout("Ready to accept connections"))
        .start()
        .await
        .unwrap()
}

pub async fn minio() -> ContainerAsync<GenericImage> {
    GenericImage::new("minio/minio", "latest")
        .with_cmd(["server", "/data", "--console-address", ":9001"])
        .with_env_var("MINIO_ROOT_USER", "minioadmin")
        .with_env_var("MINIO_ROOT_PASSWORD", "minioadmin")
        .with_exposed_port(9000)
        .with_wait_for(WaitFor::message_on_stdout("API: http://"))
        .start()
        .await
        .unwrap()
}
```

## Database migrations

```rust
// tests/support/db.rs
pub async fn migrate(pool: &PgPool) {
    sqlx::migrate!("./migrations").run(pool).await.unwrap();
}
```

Each test gets a **fresh schema** via transaction rollback:

```rust
#[sqlx::test(migrations = "../migrations")]
async fn test_crawl_queue_upsert(pool: PgPool) {
    let repo = CrawlQueueRepo::new(pool);
    repo.upsert(&[url!("https://example.com")]).await.unwrap();
    let batch = repo.claim(10).await.unwrap();
    assert_eq!(batch.len(), 1);
}
```

## API contract tests

```rust
// tests/api/search_contract.rs
use lynx_api::SearchRequest;

#[tokio::test]
async fn search_returns_200_and_valid_schema() {
    let (addr, _guard) = spawn_api().await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{addr}/v1/search"))
        .json(&SearchRequest { q: "rust".into(), ..Default::default() })
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: SearchResponse = resp.json().await.unwrap();
    assert!(body.took_ms > 0);
    assert!(body.results.len() <= 10);
}
```

## Crawler integration flow

```rust
// tests/integration/crawler_flow.rs
#[tokio::test]
async fn crawl_fetches_and_stores() {
    let (_pg, pg_pool) = spawn_postgres().await;
    let (_redis, redis_client) = spawn_redis().await;
    let (_minio, s3_client) = spawn_minio().await;

    let crawler = Crawler::new(Config {
        db: pg_pool,
        redis: redis_client,
        s3: s3_client,
        concurrency: 4,
        ..Default::default()
    });

    // Seed queue
    sqlx::query!("INSERT INTO crawl_queue (url, priority) VALUES ('https://example.com', 1)")
        .execute(&crawler.db)
        .await
        .unwrap();

    // Run one batch
    crawler.run_batch(1).await.unwrap();

    // Verify page stored in S3 + metadata in Postgres
    let page = sqlx::query_as!(Page, "SELECT * FROM pages WHERE url = $1", "https://example.com")
        .fetch_one(&crawler.db)
        .await
        .unwrap();
    assert!(page.content_hash.is_some());
    let obj = s3_client.get_object(&page.content_hash.unwrap()).await.unwrap();
    assert!(obj.body.len() > 100);
}
```

## Indexer integration

```rust
// tests/integration/indexer_build.rs
#[tokio::test]
async fn indexer_builds_generation() {
    let (_pg, pg_pool) = spawn_postgres().await;
    let index_dir = tempfile::tempdir().unwrap();

    let indexer = Indexer::new(IndexerConfig {
        db: pg_pool,
        index_path: index_dir.path().to_path_buf(),
        threads: 2,
    });

    // Insert test docs
    for i in 0..1000 {
        sqlx::query!("INSERT INTO pages (url, title, content_hash) VALUES ($1, $2, $3)",
            format!("https://example.com/{i}"), format!("Doc {i}"), format!("hash{i}"))
            .execute(&indexer.db).await.unwrap();
    }

    let gen = indexer.build_generation().await.unwrap();
    assert_eq!(gen, 1);
    assert!(index_dir.path().join("gen-1").exists());

    // Verify search works
    let reader = TantivyReader::open(index_dir.path().join("gen-1")).unwrap();
    let results = reader.search("Doc 500", 10).unwrap();
    assert_eq!(results.len(), 1);
}
```

## Running integration tests

```bash
# Requires Docker daemon
cargo nextest run --test integration -- --test-threads=4

# With debug logs
RUST_LOG=debug cargo nextest run --test integration -- --nocapture
```

## CI configuration

```yaml
# .github/workflows/integration.yml
jobs:
  integration:
    runs-on: ubuntu-latest
    services:
      docker: # Docker-in-Docker for testcontainers
        image: docker:24-dind
        options: --privileged
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - name: Run integration tests
        run: cargo nextest run --test integration
        env:
          TESTCONTAINERS_RYUK_DISABLED: "true"  # Ryuk doesn't work in DINd
```

## Data fixtures

Large fixtures in `tests/fixtures/`:

```
tests/fixtures/
  ├── corpus-10k/          # 10k real pages (CC-BY)
  │   ├── docs.jsonl
  │   └── judge-set.jsonl  # query + relevance labels
  └── crawl-warc/          # Sample WARC for parser tests
```

Load via `include_bytes!` or download in CI (cached).

## Parallelism & isolation

- Each test file gets **own container set** (spawned in `#[tokio::test]` setup).
- `--test-threads=4` default (4 CPU cores in CI).
- No shared state between tests.

## Cleanup

Testcontainers auto-removes containers on drop.
`ryuk` reaper disabled in CI; rely on Docker `--rm` + GitHub Actions runner cleanup.