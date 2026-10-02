# End-to-End Testing

## Scope

E2E tests validate **full user journeys** against a deployed stack (Kubernetes kind / Docker Compose).
They run nightly and on release candidates.

## Test scenarios

| Scenario | Description | SLA |
| -------- | ----------- | --- |
| `search_happy_path` | Query → results → suggestion click → detail | p99 < 200 ms |
| `search_unicode` | Emoji, CJK, RTL queries | no 500 |
| `search_empty` | Zero results → suggestions | < 50 ms |
| `crawl_new_domain` | Seed URL → fetch → index → searchable | < 5 min |
| `index_rollover` | Trigger build → generation swap → zero errors | < 30 s reload |
| `api_failure_recovery` | Kill API pod → requests route to healthy | < 10 s |
| `crawler_backpressure` | Queue 100k → scale workers → drain | no OOM |
| `cache_invalidation` | Update doc → index rebuild → cache miss → refresh | < 2 min |
| `tls_mtls` | All inter-service traffic encrypted | cert valid |
| `chaos_pod_kill` | Random pod kill → system self-heals | no data loss |

## Infrastructure (kind)

```yaml
# tests/e2e/kind-cluster.yaml
kind: Cluster
apiVersion: kind.x-k8s.io/v1alpha4
nodes:
  - role: control-plane
    kubeadmConfigPatches:
      - |
        kind: InitConfiguration
        nodeRegistration:
          kubeletExtraArgs:
            node-labels: "lynx.io/e2e=true"
  - role: worker
  - role: worker
```

```bash
# Spin up
kind create cluster --config tests/e2e/kind-cluster.yaml
kubectl apply -k deploy/kubernetes/overlays/e2e
# Wait for deployments
kubectl wait --for=condition=available deployment --all -n lynx --timeout=300s
```

## Test runner (Rust + kube-rs)

```rust
// tests/e2e/harness.rs
use kube::{Client, Config};
use std::sync::Arc;

pub struct E2ECluster {
    pub client: Client,
    pub api_url: String,
    pub namespace: String,
}

impl E2ECluster {
    pub async fn new() -> Self {
        let config = Config::infer().await.unwrap();
        let client = Client::try_from(config).unwrap();
        let api_url = "http://lynx-api.lynx.svc.cluster.local:8080".to_string();
        Self { client, api_url, namespace: "lynx".into() }
    }

    pub async fn restart_deployment(&self, name: &str) {
        let dp: Api<Deployment> = Api::namespaced(self.client.clone(), &self.namespace);
        dp.restart(name).await.unwrap();
        // Wait for rollout
        tokio::time::timeout(Duration::from_secs(120), async {
            loop {
                let d = dp.get(name).await.unwrap();
                if d.status.as_ref().map(|s| s.ready_replicas == Some(3)).unwrap_or(false) {
                    break;
                }
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }).await.unwrap();
    }
}
```

## Example test

```rust
// tests/e2e/search_happy_path.rs
use crate::harness::E2ECluster;
use lynx_api::SearchRequest;

#[tokio::test]
async fn search_happy_path() {
    let cluster = E2ECluster::new().await;
    let client = reqwest::Client::new();

    // 1. Search
    let resp = client
        .post(format!("{}/v1/search", cluster.api_url))
        .json(&SearchRequest { q: "rust programming".into(), ..Default::default() })
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: SearchResponse = resp.json().await.unwrap();
    assert!(!body.results.is_empty());
    let first_url = body.results[0].url.clone();

    // 2. Suggestions for prefix
    let sugg = client
        .get(format!("{}/v1/suggest?q=rust", cluster.api_url))
        .send()
        .await
        .unwrap();
    assert_eq!(sugg.status(), 200);

    // 3. Verify result is actually crawlable (not just indexed)
    let head = client.head(&first_url).send().await.unwrap();
    assert!(head.status().is_success());
}
```

## Chaos testing

```rust
// tests/e2e/chaos_pod_kill.rs
#[tokio::test]
async fn chaos_kill_api_pod() {
    let cluster = E2ECluster::new().await;
    let client = reqwest::Client::new();

    // Baseline
    let baseline = latency_p50(&client, &cluster.api_url).await;

    // Kill one API pod
    let pods = cluster.list_pods("app=lynx-api").await;
    cluster.delete_pod(&pods[0]).await;

    // Wait for recovery
    tokio::time::sleep(Duration::from_secs(15)).await;

    // Verify latency stable
    let after = latency_p50(&client, &cluster.api_url).await;
    assert!(after < baseline * 1.5); // < 50% degradation
}
```

## Performance baselines

```rust
// tests/e2e/perf_baseline.rs
#[tokio::test]
#[ignore = "run manually with --ignored"]
async fn perf_baseline_100k_qps() {
    let cluster = E2ECluster::new().await;
    let results = load_test::run(LoadConfig {
        url: cluster.api_url.clone(),
        qps: 100_000,
        duration: Duration::from_secs(300),
        queries: load_test::QUERY_MIX.clone(),
    }).await;

    assert!(results.p99_latency_ms < 200);
    assert!(results.error_rate < 0.001);
    assert!(results.throughput_qps > 95_000);
}
```

## Running E2E

```bash
# Local (kind)
make e2e-setup          # creates cluster, deploys
cargo nextest run --test e2e -- --ignored
make e2e-teardown

# CI (nightly)
# .github/workflows/e2e.yml runs on schedule
```

## CI pipeline

```yaml
# .github/workflows/e2e.yml
on:
  schedule:
    - cron: '0 2 * * *'  # 02:00 UTC daily
  workflow_dispatch:

jobs:
  e2e:
    runs-on: ubuntu-latest
    timeout-minutes: 60
    steps:
      - uses: actions/checkout@v4
      - name: Create kind cluster
        run: make e2e-setup
      - name: Run E2E tests
        run: cargo nextest run --test e2e -- --ignored
      - name: Upload artifacts
        if: failure()
        uses: actions/upload-artifact@v4
        with:
          name: e2e-logs
          path: |
            /tmp/kind-logs/
            target/nextest/
      - name: Teardown
        if: always()
        run: make e2e-teardown
```

## Artifacts on failure

- Kind cluster logs (`kind export logs`)
- Pod logs (`kubectl logs -l app=lynx-api --tail=1000`)
- Nextest JUnit XML
- Heap profiles (if `pprof` enabled)

## Flaky test handling

Same as unit: quarantine + issue. E2E flakes get higher priority (block release).