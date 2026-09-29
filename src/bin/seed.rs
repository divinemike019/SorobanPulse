//! Seed-data generator for local development and demos (Issue #1148).
//!
//! Inserts realistic Soroban events spanning 30 days, sample subscriptions,
//! notification channels, and contract labels so that the dashboard and API
//! show meaningful data immediately after `make seed`.
//!
//! Usage:
//!   cargo run --bin seed -- [OPTIONS]
//!
//! Options:
//!   --events <N>            Total number of events to insert (default: 200)
//!   --database-url <URL>    PostgreSQL connection string
//!                           (falls back to DATABASE_URL env var)
//!   --no-subscriptions      Skip subscription rows
//!   --no-channels           Skip notification channel rows
//!   --no-metadata           Skip contract_metadata rows
//!   -h, --help              Print this message

use chrono::{DateTime, Duration, Utc};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use serde_json::{json, Value};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::process::ExitCode;
use uuid::Uuid;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_usage();
        return ExitCode::SUCCESS;
    }

    let event_count: usize = flag_value(&args, "--events")
        .and_then(|v| v.parse().ok())
        .unwrap_or(200);

    let database_url = flag_value(&args, "--database-url")
        .or_else(|| std::env::var("DATABASE_URL").ok())
        .unwrap_or_else(|| {
            eprintln!("error: --database-url or DATABASE_URL must be set");
            std::process::exit(1);
        });

    let seed_subscriptions = !args.iter().any(|a| a == "--no-subscriptions");
    let seed_channels = !args.iter().any(|a| a == "--no-channels");
    let seed_metadata = !args.iter().any(|a| a == "--no-metadata");

    // Single-threaded Tokio runtime keeps the binary lightweight.
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("failed to build Tokio runtime");

    rt.block_on(async move {
        match run(
            &database_url,
            event_count,
            seed_subscriptions,
            seed_channels,
            seed_metadata,
        )
        .await
        {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::FAILURE
            }
        }
    })
}

// ── Core logic ────────────────────────────────────────────────────────────────

async fn run(
    database_url: &str,
    event_count: usize,
    seed_subscriptions: bool,
    seed_channels: bool,
    seed_metadata: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    eprintln!("[seed] connecting to database…");
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(database_url)
        .await?;

    eprintln!("[seed] inserting {event_count} events…");
    let events_inserted = insert_events(&pool, event_count).await?;
    eprintln!("[seed] ✓ {events_inserted} events inserted");

    if seed_metadata {
        eprintln!("[seed] inserting contract_metadata…");
        let n = insert_contract_metadata(&pool).await?;
        eprintln!("[seed] ✓ {n} contract_metadata rows inserted");
    }

    if seed_subscriptions {
        eprintln!("[seed] inserting subscriptions…");
        let n = insert_subscriptions(&pool).await?;
        eprintln!("[seed] ✓ {n} subscriptions inserted");
    }

    if seed_channels {
        eprintln!("[seed] inserting notification_channels…");
        let n = insert_notification_channels(&pool).await?;
        eprintln!("[seed] ✓ {n} notification channels inserted");
    }

    let total_events: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM events")
        .fetch_one(&pool)
        .await?;
    eprintln!("[seed] ✓ done — {total_events} total events in database");
    Ok(())
}

// ── Events ────────────────────────────────────────────────────────────────────

/// Well-known demo contract addresses (C… = Stellar contract address style, 56 chars).
const CONTRACTS: &[(&str, &str)] = &[
    (
        "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAFCT4",
        "sep41-token",
    ),
    (
        "CBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBFCT4",
        "amm-pool",
    ),
    (
        "CCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCFCT4",
        "lending-protocol",
    ),
    (
        "CDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDFCT4",
        "nft-marketplace",
    ),
    (
        "CEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEFCT4",
        "governance",
    ),
];

/// Event-type distribution (type, weight). Weights must sum to 100.
const EVENT_TYPES: &[(&str, u32)] = &[
    ("contract", 70),   // 70 % contract events
    ("diagnostic", 20), // 20 % diagnostic events
    ("system", 10),     // 10 % system events
];

/// SEP-41 and other realistic Soroban event topics.
const TOPICS: &[&str] = &[
    "transfer",
    "mint",
    "burn",
    "approve",
    "swap",
    "add_liquidity",
    "remove_liquidity",
    "borrow",
    "repay",
    "liquidate",
    "stake",
    "unstake",
    "vote",
    "propose",
    "log",
    "fee",
];

fn sample_event_data(rng: &mut StdRng, topic: &str, contract_label: &str) -> Value {
    let amount: u64 = rng.gen_range(1_000_u64..100_000_000_u64);
    let from_addr = format!("GADDR{:052}", rng.gen::<u32>());
    let to_addr = format!("GADDR{:052}", rng.gen::<u32>());

    match topic {
        "transfer" | "mint" | "burn" => json!({
            "topic": [topic, &from_addr, &to_addr],
            "value": { "i128": { "hi": 0, "lo": amount } }
        }),
        "swap" => {
            let amount_out = amount / 7;
            json!({
                "topic": ["swap", contract_label],
                "value": {
                    "token_in":  "XLM",
                    "token_out": "USDC",
                    "amount_in":  { "i128": { "hi": 0, "lo": amount } },
                    "amount_out": { "i128": { "hi": 0, "lo": amount_out } },
                    "user": &from_addr
                }
            })
        }
        "approve" => json!({
            "topic": ["approve", &from_addr, &to_addr],
            "value": { "i128": { "hi": 0, "lo": amount } }
        }),
        "add_liquidity" | "remove_liquidity" => {
            let amount_b = amount / 7;
            let shares = amount / 100;
            json!({
                "topic": [topic, &from_addr],
                "value": {
                    "amount_a": { "i128": { "hi": 0, "lo": amount } },
                    "amount_b": { "i128": { "hi": 0, "lo": amount_b } },
                    "shares":   { "i128": { "hi": 0, "lo": shares } }
                }
            })
        }
        "borrow" | "repay" => json!({
            "topic": [topic, &from_addr],
            "value": {
                "asset":  "USDC",
                "amount": { "i128": { "hi": 0, "lo": amount } }
            }
        }),
        "liquidate" => json!({
            "topic": ["liquidate", &from_addr, &to_addr],
            "value": {
                "collateral_seized": { "i128": { "hi": 0, "lo": amount } },
                "debt_repaid":       { "i128": { "hi": 0, "lo": amount / 2 } }
            }
        }),
        "stake" | "unstake" => json!({
            "topic": [topic, &from_addr],
            "value": { "i128": { "hi": 0, "lo": amount } }
        }),
        "vote" | "propose" => {
            let proposal_id: u32 = rng.gen_range(1_u32..1_000_u32);
            let support: bool = rng.gen_bool(0.6);
            json!({
                "topic": [topic, &from_addr],
                "value": {
                    "proposal_id": proposal_id,
                    "support":     support
                }
            })
        }
        "log" => {
            let seq: u16 = rng.gen();
            json!({
                "topic": ["log"],
                "value": { "string": format!("diagnostic:{contract_label}-{seq}") }
            })
        }
        "fee" => {
            let fee: u32 = rng.gen_range(100_u32..10_000_u32);
            json!({
                "topic": ["fee"],
                "value": { "amount": { "i128": { "hi": 0, "lo": fee } } }
            })
        }
        _ => {
            let raw: u64 = rng.gen();
            json!({
                "topic": [topic],
                "value": { "raw": format!("{raw:016x}") }
            })
        }
    }
}

/// Weighted random selection from a `(item, weight)` slice.
fn pick_weighted<'a>(rng: &mut StdRng, choices: &[(&'a str, u32)]) -> &'a str {
    let total: u32 = choices.iter().map(|(_, w)| *w).sum();
    let mut roll: u32 = rng.gen_range(0..total);
    for (item, weight) in choices {
        if roll < *weight {
            return item;
        }
        roll -= weight;
    }
    choices.last().unwrap().0
}

async fn insert_events(pool: &PgPool, count: usize) -> Result<usize, sqlx::Error> {
    let mut rng = StdRng::seed_from_u64(0xDEAD_BEEF_1148_u64);

    let now = Utc::now();
    // Spread events uniformly over the last 30 days.
    let window_secs: i64 = 30 * 24 * 3600;
    // Ledger ~5 s each; start 30 days' worth of ledgers back.
    let base_ledger: i64 = 1_000_000_i64;
    let ledgers_per_window: i64 = window_secs / 5;

    let mut inserted = 0usize;

    // Process in batches so progress is visible and memory stays bounded.
    const BATCH: usize = 500;

    while inserted < count {
        let batch_size = (count - inserted).min(BATCH);
        let mut tx = pool.begin().await?;

        for batch_idx in 0..batch_size {
            let global_idx = inserted + batch_idx;

            let (contract_id, contract_label) =
                CONTRACTS[rng.gen_range(0..CONTRACTS.len())];
            let event_type = pick_weighted(&mut rng, EVENT_TYPES);
            let topic = TOPICS[rng.gen_range(0..TOPICS.len())];
            let event_data = sample_event_data(&mut rng, topic, contract_label);

            // Random timestamp within the last 30 days.
            let secs_ago: i64 = rng.gen_range(0_i64..window_secs);
            let timestamp: DateTime<Utc> = now - Duration::seconds(secs_ago);

            // Matching ledger (older timestamp → lower ledger number).
            let ledger_offset = (secs_ago * ledgers_per_window) / window_secs;
            let ledger: i64 = base_ledger + ledgers_per_window - ledger_offset;

            // Deterministic-ish 64-char hex tx hash; XOR with index for uniqueness.
            let rand_hi: u64 = rng.gen();
            let rand_lo: u64 = rng.gen();
            let tx_hash = format!(
                "{:032x}{:032x}",
                u128::from(rand_hi) ^ (global_idx as u128),
                u128::from(rand_lo)
            );

            let fp_rand: u64 = rng.gen();
            let fingerprint = format!(
                "{:032x}{:032x}",
                u128::from(fp_rand) ^ (ledger.unsigned_abs() as u128),
                ledger.unsigned_abs() as u128
            );

            sqlx::query(
                r#"
                INSERT INTO events (
                    id, contract_id, event_type, tx_hash, ledger,
                    timestamp, event_data, created_at,
                    schema_version, anonymized, fingerprint,
                    tenant_id, network, in_successful_call
                )
                VALUES (
                    $1, $2, $3, $4, $5,
                    $6, $7, NOW(),
                    1, FALSE, $8,
                    'default', 'soroban-testnet', TRUE
                )
                ON CONFLICT DO NOTHING
                "#,
            )
            .bind(Uuid::new_v4())
            .bind(contract_id)
            .bind(event_type)
            .bind(&tx_hash)
            .bind(ledger)
            .bind(timestamp)
            .bind(&event_data)
            .bind(&fingerprint)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        inserted += batch_size;
        eprint!("\r[seed]   {inserted}/{count} events…   ");
    }
    eprintln!();
    Ok(inserted)
}

// ── Contract metadata ─────────────────────────────────────────────────────────

async fn insert_contract_metadata(pool: &PgPool) -> Result<usize, sqlx::Error> {
    struct Entry {
        contract_id: &'static str,
        name: &'static str,
        description: &'static str,
        tags: &'static [&'static str],
    }

    const ENTRIES: &[Entry] = &[
        Entry {
            contract_id: CONTRACTS[0].0,
            name: "SEP-41 Demo Token",
            description: "A SEP-41 compliant fungible token used for development demos.",
            tags: &["token", "sep41", "fungible"],
        },
        Entry {
            contract_id: CONTRACTS[1].0,
            name: "AMM Pool",
            description: "Automated market-maker liquidity pool.",
            tags: &["defi", "amm", "liquidity"],
        },
        Entry {
            contract_id: CONTRACTS[2].0,
            name: "Lending Protocol",
            description: "Over-collateralised lending and borrowing platform.",
            tags: &["defi", "lending", "collateral"],
        },
        Entry {
            contract_id: CONTRACTS[3].0,
            name: "NFT Marketplace",
            description: "On-chain NFT minting and trading contract.",
            tags: &["nft", "marketplace", "collectibles"],
        },
        Entry {
            contract_id: CONTRACTS[4].0,
            name: "Governance",
            description: "On-chain governance with proposal voting.",
            tags: &["governance", "dao", "voting"],
        },
    ];

    let mut n = 0usize;
    for entry in ENTRIES {
        let tag_vec: Vec<&str> = entry.tags.to_vec();
        sqlx::query(
            r#"
            INSERT INTO contract_metadata (
                contract_id, name, description,
                project_url, source_repo, tags, verified,
                created_at, updated_at
            )
            VALUES (
                $1, $2, $3,
                'https://sorobanpulse.example', 'https://github.com/example/contract',
                $4, TRUE,
                NOW(), NOW()
            )
            ON CONFLICT (contract_id) DO UPDATE
                SET name        = EXCLUDED.name,
                    description = EXCLUDED.description,
                    tags        = EXCLUDED.tags,
                    updated_at  = NOW()
            "#,
        )
        .bind(entry.contract_id)
        .bind(entry.name)
        .bind(entry.description)
        .bind(&tag_vec)
        .execute(pool)
        .await?;

        n += 1;
    }
    Ok(n)
}

// ── Subscriptions ─────────────────────────────────────────────────────────────

async fn insert_subscriptions(pool: &PgPool) -> Result<usize, sqlx::Error> {
    const ENTRIES: &[(&str, i64)] = &[
        ("https://webhook.example.com/soroban/all", 1_000_000),
        ("https://webhook.example.com/soroban/token", 1_000_000),
        ("https://webhook.example.com/soroban/defi", 1_010_000),
        ("https://ci.example.internal/hooks/soroban", 1_020_000),
        ("https://alerts.example.org/pulse", 1_020_000),
    ];

    let mut n = 0usize;
    for (callback_url, from_ledger) in ENTRIES {
        sqlx::query(
            r#"
            INSERT INTO subscriptions
                (id, callback_url, from_ledger, acked_ledger, status, created_at)
            VALUES
                (gen_random_uuid(), $1, $2, $2, 'active', NOW())
            ON CONFLICT DO NOTHING
            "#,
        )
        .bind(callback_url)
        .bind(from_ledger)
        .execute(pool)
        .await?;

        n += 1;
    }
    Ok(n)
}

// ── Notification channels ─────────────────────────────────────────────────────

async fn insert_notification_channels(pool: &PgPool) -> Result<usize, sqlx::Error> {
    let default_retry = json!({
        "max_attempts": 3,
        "initial_backoff_ms": 1000,
        "backoff_multiplier": 2.0,
        "max_backoff_ms": 60000
    });

    struct Channel {
        name: &'static str,
        channel_type: &'static str,
        config: Value,
    }

    let channels = vec![
        Channel {
            name: "ops-webhook",
            channel_type: "webhook",
            config: json!({
                "url": "https://hooks.example.com/soroban-pulse",
                "secret": "dev-only-secret-change-me"
            }),
        },
        Channel {
            name: "dev-email",
            channel_type: "email",
            config: json!({
                "to": ["dev@example.com"],
                "from": "noreply@sorobanpulse.example",
                "subject_template": "[SorobanPulse] New event on {{contract_id}}"
            }),
        },
        Channel {
            name: "pagerduty-critical",
            channel_type: "webhook",
            config: json!({
                "url": "https://events.pagerduty.com/v2/enqueue",
                "routing_key": "dev-pagerduty-routing-key"
            }),
        },
        Channel {
            name: "sms-oncall",
            channel_type: "sms",
            config: json!({
                "to": ["+15550001234"],
                "provider": "twilio",
                "account_sid": "dev-account-sid"
            }),
        },
    ];

    let mut n = 0usize;
    for ch in &channels {
        sqlx::query(
            r#"
            INSERT INTO notification_channels
                (id, name, channel_type, config, retry_policy, created_at, updated_at)
            VALUES
                (gen_random_uuid(), $1, $2, $3, $4, NOW(), NOW())
            ON CONFLICT (name) DO UPDATE
                SET config     = EXCLUDED.config,
                    updated_at = NOW()
            "#,
        )
        .bind(ch.name)
        .bind(ch.channel_type)
        .bind(&ch.config)
        .bind(&default_retry)
        .execute(pool)
        .await?;

        n += 1;
    }
    Ok(n)
}

// ── CLI helpers ───────────────────────────────────────────────────────────────

fn flag_value(args: &[String], flag: &str) -> Option<String> {
    args.windows(2).find(|w| w[0] == flag).map(|w| w[1].clone())
}

fn print_usage() {
    eprintln!(
        "Usage: seed [OPTIONS]

Populates a fresh SorobanPulse database with realistic development data.

Options:
  --events <N>          Number of events to insert (default: 200)
  --database-url <URL>  PostgreSQL connection string
                        (falls back to DATABASE_URL env var)
  --no-subscriptions    Skip subscription rows
  --no-channels         Skip notification channel rows
  --no-metadata         Skip contract_metadata rows
  -h, --help            Show this message

Examples:
  # Quick dev seed (200 events)
  cargo run --bin seed

  # Performance / benchmark dataset
  cargo run --bin seed -- --events 100000

  # Custom DB, events only
  cargo run --bin seed -- \\
    --database-url postgres://user:pass@localhost/soroban_pulse \\
    --events 500 \\
    --no-subscriptions --no-channels"
    );
}
