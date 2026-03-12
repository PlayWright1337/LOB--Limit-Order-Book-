# LOB Engine

A Rust limit order book engine with deterministic event sourcing, self-trade prevention, fast order cancellation, and lock-free snapshot reads via double buffering.

This project implements a focused matching core for a single instrument:
- limit orders with FIFO execution inside each price level
- market orders
- fast cancel by `OrderId`
- deterministic replay from state events
- Criterion benchmarks for core paths

## Features

- Integer-only price and quantity model, no `f64`
- `BTreeMap` price ladder for bids and asks
- `VecDeque<Order>` per price level with aggregated `total_volume`
- `SmallVec<[Execution; 4]>` on the hot path
- `OrderId -> OrderLocator` index for fast cancel
- self-trade prevention by `participant_id`
- lock-free snapshot reads with `ArcSwap`
- deterministic `replay(events)` from state transitions

## Tech Stack

- **Language**: Rust stable, edition 2024
- **Async Runtime**: Tokio
- **Serialization**: Serde
- **Performance Helper**: SmallVec
- **Snapshot Publication**: ArcSwap
- **Locking**: parking_lot
- **Benchmarking**: Criterion

## Project Layout

```text
.
├── Cargo.toml
├── README.md
├── benches
│   └── matching_bench.rs
└── src
    ├── book.rs
    ├── concurrent.rs
    ├── lib.rs
    └── types.rs
```

## Core Types

```rust
pub type Price = u64;
pub type Quantity = u64;
pub type OrderId = u64;
pub type ParticipantId = u32;
```

`Price` is intended to be fixed-point at the integration boundary. For example, `100_00000000` can represent `100.0` when using 8 decimal places.

## Data Model

### Order

An `Order` stores:
- `id`
- `participant_id`
- `side`
- `price`
- `quantity`

### PriceLevel

Each price level stores:
- `VecDeque<Order>` for FIFO order preservation
- `total_volume` for fast level aggregation
- `head_offset` for lazy tombstone compaction and stable indexing

### OrderBook

The book contains:
- `bids: BTreeMap<Price, PriceLevel>`
- `asks: BTreeMap<Price, PriceLevel>`
- `order_index: HashMap<OrderId, OrderLocator>`
- `event_log: Vec<Event>`

## Matching Rules

### Limit Orders

`submit_limit()`:

1. Validates `order_id` and `quantity`
2. Looks at the best opposite-side price
3. Checks whether the incoming price crosses
4. Checks self-trade prevention
5. Emits one or more `Execution`s
6. Leaves any remainder resting on the book

### Market Orders

`submit_market()`:

1. Validates the request
2. Walks the best opposite-side levels
3. Executes until fully filled or the book is empty
4. Never rests on the book

### Cancels

`cancel(order_id)`:

1. Resolves the order through `order_index`
2. Jumps directly to the target price level
3. Marks the order as removed by setting `quantity = 0`
4. Decrements `total_volume`
5. Compacts the front of the deque lazily

This keeps cancel fast without scanning the full queue.

## Self-Trade Prevention

Before consuming opposite-side liquidity, the engine compares `participant_id`:

```rust
if resting.participant_id == request.participant_id {
    return Err(OrderBookError::SelfTrade {
        resting_order_id: resting.id,
        incoming_order_id: request.id,
    });
}
```

## Event Sourcing

The engine records state events:

- `Event::Accepted(Order)`
- `Event::Executed(Execution)`
- `Event::Cancelled(CancelledOrder)`

This is important: `replay()` rebuilds the book by applying state transitions, not by rerunning matching logic.

That gives you:
- deterministic restoration
- easy auditing
- a clean state log
- straightforward serialization and external persistence

### Replay

```rust
let rebuilt = OrderBook::replay(events)?;
```

`replay()` accepts any `IntoIterator<Item = Event>`, so it works with:
- arrays
- `Vec<Event>`
- iterators
- deserialized event streams

## Concurrent Snapshots

`ConcurrentOrderBook` uses a double-buffered pattern:

- writers mutate a `Mutex<OrderBook>`
- after each write, a fresh `Arc<OrderBook>` snapshot is published through `ArcSwap`
- readers fetch the latest immutable snapshot without taking the writer lock

This favors:
- cheap UI/API reads
- predictable snapshot semantics
- minimal reader interference with matching

## Public API

### `OrderBook`

- `submit_limit(NewOrderRequest) -> Result<SubmissionOutcome, OrderBookError>`
- `submit_market(MarketOrderRequest) -> Result<SubmissionOutcome, OrderBookError>`
- `cancel(OrderId) -> Result<CancelledOrder, OrderBookError>`
- `replay(events) -> Result<OrderBook, OrderBookError>`
- `apply(Event) -> Result<(), OrderBookError>`
- `best_bid() -> Option<(Price, Quantity)>`
- `best_ask() -> Option<(Price, Quantity)>`
- `total_resting_orders() -> usize`
- `event_log() -> &[Event]`

### `ConcurrentOrderBook`

- `submit_limit(...)`
- `submit_market(...)`
- `cancel(...)`
- `snapshot() -> Arc<OrderBook>`

## Error Handling

The main error type is `OrderBookError`.

Supported errors:
- `DuplicateOrderId`
- `InvalidQuantity`
- `UnknownOrder`
- `SelfTrade`
- `ReplayInvariantBroken`

Public matching logic returns `Result` instead of panicking.

## Getting Started

### Prerequisites

- Rust stable
- Cargo

Check your toolchain:

```bash
rustc --version
cargo --version
```

### Build

```bash
cargo build
```

### Run Tests

```bash
cargo test
```

### Run Clippy

```bash
cargo clippy --all-targets --all-features -- -D warnings
```

### Run Benchmarks

```bash
cargo bench --bench matching_bench
```

Quick benchmark mode:

```bash
cargo bench --bench matching_bench -- --quick
```

## Example

```rust
use lob_engine::{MarketOrderRequest, NewOrderRequest, OrderBook, Side};

fn main() -> Result<(), lob_engine::OrderBookError> {
    let mut book = OrderBook::default();

    book.submit_limit(NewOrderRequest {
        id: 1,
        participant_id: 10,
        side: Side::Ask,
        price: 100,
        quantity: 5,
    })?;

    let outcome = book.submit_limit(NewOrderRequest {
        id: 2,
        participant_id: 20,
        side: Side::Bid,
        price: 101,
        quantity: 3,
    })?;

    assert_eq!(outcome.executions.len(), 1);
    assert_eq!(book.best_ask(), Some((100, 2)));

    let market = book.submit_market(MarketOrderRequest {
        id: 3,
        participant_id: 30,
        side: Side::Bid,
        quantity: 2,
    })?;

    assert_eq!(market.executions.len(), 1);
    assert_eq!(book.best_ask(), None);

    Ok(())
}
```

## Test Coverage

The current test suite covers:
- resting limit orders
- FIFO matching inside the same level
- multi-level market consumption
- partial fills
- cancel existing order
- cancel missing order
- self-trade prevention
- replay from event log
- concurrent snapshot publication
- event serialization

## Benchmark Results

Latest quick benchmark on this machine:

- `limit_order_insertion/empty_book`: about `4.3 M ops/s`
- `limit_order_match/full_cross`: about `1.77 M ops/s`
- `cancel_order/middle_of_level`: about `0.52 M ops/s`

These numbers are real Criterion outputs from:

```bash
cargo bench --bench matching_bench -- --quick
```

## Current Limitations

- single-instrument book
- no network API yet
- no built-in disk persistence
- replay operates on `Event`, not a binary log reader
- snapshot publication clones the full book after each write

That last point is a deliberate tradeoff for simplicity and clean read semantics. It is good for correctness and readability, but it is not yet the final ultra-low-latency design.

## Possible Next Steps

- binary event log and replay from bytes
- L2 and L3 snapshot APIs
- Tokio-based gateway
- multi-instrument engine
- metrics and tracing
- property-based tests
- fuzzing for order book invariants

## Development Commands

```bash
cargo build
cargo test
cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo bench --bench matching_bench -- --quick
```

## License

No license file is currently included. If this project is going public, add `MIT`, `Apache-2.0`, or a dual-license setup explicitly.
