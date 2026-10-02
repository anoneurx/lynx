# Research Papers & References

Key papers influencing LYNX design.

## Search & Ranking

| Paper | Year | LYNX Application |
| ----- | ---- | ---------------- |
| **The Probabilistic Relevance Framework: BM25 and Beyond** (Robertson & Zaragoza) | 2009 | Core BM25 implementation |
| **Learning to Rank for Information Retrieval** (Liu) | 2009 | LambdaMART pipeline |
| **LambdaMART: A Machine Learning Approach to Ranking** (Burges) | 2010 | LTR model |
| **From Frequency to Meaning: Vector Space Models of Semantics** (Turney & Pantel) | 2010 | Future: dense retrieval |
| **Dense Passage Retrieval for Open-Domain Question Answering** (Karpukhin et al.) | 2020 | Future: hybrid search |
| **ColBERT: Efficient and Effective Passage Search via Contextualized Late Interaction** (Khattab & Zaharia) | 2020 | Future: late interaction |

## Indexing & Storage

| Paper | Year | LYNX Application |
| ----- | ---- | ---------------- |
| **Managing Gigabytes: Compressing and Indexing Documents and Images** (Witten et al.) | 1999 | Tantivy design principles |
| **The Lucene Search Engine: Architecture and Implementation** (Hatcher & Gospodnetic) | 2004 | Tantivy architecture |
| **WAND: Weak AND for Efficient Top-k Query Processing** (Broder et al.) | 2003 | Tantivy WAND optimization |
| **Block-Max WAND: Faster Top-k Retrieval** (Ding & Suel) | 2011 | Tantivy block-max |
| **SIGIR 2020 Tutorial: Learned Index Structures** (Kraska et al.) | 2020 | Future: learned indexes |

## Crawling & Web Architecture

| Paper | Year | LYNX Application |
| ----- | ---- | ---------------- |
| **Efficient Crawling Through URL Ordering** (Najork & Wiener) | 2001 | Priority queue design |
| **Mercator: A Scalable, Extensible Web Crawler** (Heydon & Najork) | 1999 | Crawler architecture |
| **Politeness in Web Crawling** (Castillo et al.) | 2004 | Politeness token bucket |
| **Detecting Near-Duplicates for Web Crawling** (Manku et al.) | 2007 | TLSH/Simhash dedup |
| **The WebCache Project: A Platform for Web Caching Research** (Wessels & Duane) | 1997 | Cache design |

## Systems & Operations

| Paper | Year | LYNX Application |
| ----- | ---- | ---------------- |
| **The Tail at Scale** (Dean & Barroso) | 2013 | Latency SLOs, hedged requests |
| **Site Reliability Engineering** (Beyer et al.) | 2016 | SLOs, error budgets, runbooks |
| **Building Secure and Reliable Systems** (Heinrich et al.) | 2020 | Security runbooks |
| **Zero-Downtime Deployments at Scale** (Netflix Tech Blog) | 2018 | Rolling updates, canary |
| **Chaos Engineering** (Basiri et al.) | 2016 | E2E chaos tests |

## Privacy & Security

| Paper | Year | LYNX Application |
| ----- | ---- | ---------------- |
| **k-Anonymity: A Model for Protecting Privacy** (Sweeney) | 2002 | No query logging |
| **Differential Privacy** (Dwork) | 2006 | Future: DP metrics |
| **The Security of the TLS Protocol** (Rescorla) | 2018 | mTLS, cert-manager |

## LYNX Publications (Internal)

| Title | Authors | Year | Link |
| ----- | ------- | ---- | ---- |
| **LYNX: A Rust-Native Search Engine Architecture** | LYNX Team | 2025 | `docs/research/papers/lynx-architecture.pdf` |
| **Tantivy at Scale: Lessons from 1B Documents** | Search Team | 2025 | `docs/research/papers/tantivy-at-scale.pdf` |
| **Adversarial Robustness of HTML Parsers** | Security Team | 2024 | `docs/research/papers/html-parser-fuzzing.pdf` |

## Recommended Reading Order

1. **Start here**: Robertson & Zaragoza (BM25), Liu (LTR)
2. **Indexing**: Witten et al. (Managing Gigabytes)
3. **Systems**: Dean & Barroso (Tail at Scale), SRE Book
4. **Crawling**: Heydon & Najork (Mercator)
5. **Security**: Sweeney (k-Anonymity), TLS 1.3 RFC