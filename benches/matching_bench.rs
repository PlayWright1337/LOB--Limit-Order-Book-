use std::hint::black_box;

use criterion::{BatchSize, Criterion, Throughput, criterion_group, criterion_main};
use lob_engine::{NewOrderRequest, OrderBook, Side};

fn bench_limit_order_insertion(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("limit_order_insertion");
    group.throughput(Throughput::Elements(1));
    group.bench_function("empty_book", |bencher| {
        bencher.iter_batched(
            OrderBook::default,
            |mut book| {
                let _ = book.submit_limit(black_box(NewOrderRequest {
                    id: 1,
                    participant_id: 1,
                    side: Side::Bid,
                    price: 100,
                    quantity: 10,
                }));
            },
            BatchSize::SmallInput,
        );
    });
    group.finish();
}

fn bench_limit_order_match(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("limit_order_match");
    group.throughput(Throughput::Elements(1));
    group.bench_function("full_cross", |bencher| {
        bencher.iter_batched(
            || {
                let mut book = OrderBook::default();
                let _ = book.submit_limit(NewOrderRequest {
                    id: 1,
                    participant_id: 1,
                    side: Side::Ask,
                    price: 100,
                    quantity: 10,
                });
                book
            },
            |mut book| {
                let _ = book.submit_limit(black_box(NewOrderRequest {
                    id: 2,
                    participant_id: 2,
                    side: Side::Bid,
                    price: 100,
                    quantity: 10,
                }));
            },
            BatchSize::SmallInput,
        );
    });
    group.finish();
}

fn bench_cancel_order(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("cancel_order");
    group.throughput(Throughput::Elements(1));
    group.bench_function("middle_of_level", |bencher| {
        bencher.iter_batched(
            || {
                let mut book = OrderBook::default();
                for id in 1..=64 {
                    let _ = book.submit_limit(NewOrderRequest {
                        id,
                        participant_id: id as u32,
                        side: Side::Bid,
                        price: 100,
                        quantity: 1,
                    });
                }
                book
            },
            |mut book| {
                let _ = book.cancel(black_box(32));
            },
            BatchSize::SmallInput,
        );
    });
    group.finish();
}

criterion_group!(
    benches,
    bench_limit_order_insertion,
    bench_limit_order_match,
    bench_cancel_order
);
criterion_main!(benches);
