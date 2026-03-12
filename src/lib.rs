mod book;
mod concurrent;
mod types;

pub use book::{OrderBook, PriceLevel};
pub use concurrent::ConcurrentOrderBook;
pub use types::{
    CancelledOrder, Event, Execution, ExecutionVec, MarketOrderRequest, NewOrderRequest, Order,
    OrderBookError, OrderId, ParticipantId, Price, Quantity, Side, SubmissionOutcome,
};

#[cfg(test)]
mod tests {
    use crate::{
        ConcurrentOrderBook, Event, Execution, MarketOrderRequest, NewOrderRequest, Order,
        OrderBook, OrderBookError, Side,
    };

    #[test]
    fn limit_order_rests_when_book_is_empty() {
        let mut book = OrderBook::default();
        let result = book
            .submit_limit(NewOrderRequest {
                id: 1,
                participant_id: 7,
                side: Side::Bid,
                price: 101,
                quantity: 10,
            })
            .expect("limit order should be accepted");

        assert!(result.executions.is_empty());
        assert_eq!(book.best_bid(), Some((101, 10)));
        assert_eq!(book.total_resting_orders(), 1);
    }

    #[test]
    fn crossing_limit_order_matches_in_fifo_order() {
        let mut book = OrderBook::default();
        let _ = book.submit_limit(NewOrderRequest {
            id: 1,
            participant_id: 10,
            side: Side::Ask,
            price: 100,
            quantity: 5,
        });
        let _ = book.submit_limit(NewOrderRequest {
            id: 2,
            participant_id: 11,
            side: Side::Ask,
            price: 100,
            quantity: 7,
        });

        let result = book
            .submit_limit(NewOrderRequest {
                id: 3,
                participant_id: 12,
                side: Side::Bid,
                price: 101,
                quantity: 8,
            })
            .expect("crossing order should match");

        assert_eq!(
            result.executions.as_slice(),
            &[Execution::new(1, 3, 100, 5), Execution::new(2, 3, 100, 3),]
        );
        assert_eq!(book.best_ask(), Some((100, 4)));
        assert_eq!(book.total_resting_orders(), 1);
    }

    #[test]
    fn market_order_consumes_multiple_levels() {
        let mut book = OrderBook::default();
        let _ = book.submit_limit(NewOrderRequest {
            id: 1,
            participant_id: 1,
            side: Side::Ask,
            price: 100,
            quantity: 5,
        });
        let _ = book.submit_limit(NewOrderRequest {
            id: 2,
            participant_id: 2,
            side: Side::Ask,
            price: 101,
            quantity: 5,
        });

        let result = book
            .submit_market(MarketOrderRequest {
                id: 10,
                participant_id: 3,
                side: Side::Bid,
                quantity: 8,
            })
            .expect("market order should match");

        assert_eq!(
            result.executions.as_slice(),
            &[Execution::new(1, 10, 100, 5), Execution::new(2, 10, 101, 3),]
        );
        assert_eq!(book.best_ask(), Some((101, 2)));
    }

    #[test]
    fn cancel_existing_order_removes_it_from_book() {
        let mut book = OrderBook::default();
        let _ = book.submit_limit(NewOrderRequest {
            id: 1,
            participant_id: 42,
            side: Side::Bid,
            price: 99,
            quantity: 10,
        });

        let cancelled = book.cancel(1).expect("order should be cancelled");

        assert_eq!(cancelled.id, 1);
        assert_eq!(book.best_bid(), None);
        assert_eq!(book.total_resting_orders(), 0);
    }

    #[test]
    fn cancelling_missing_order_returns_error() {
        let mut book = OrderBook::default();
        let error = book.cancel(999).expect_err("missing order must error");

        assert_eq!(error, OrderBookError::UnknownOrder(999));
    }

    #[test]
    fn partial_fill_leaves_residual_on_book() {
        let mut book = OrderBook::default();
        let _ = book.submit_limit(NewOrderRequest {
            id: 1,
            participant_id: 1,
            side: Side::Ask,
            price: 100,
            quantity: 10,
        });

        let result = book
            .submit_limit(NewOrderRequest {
                id: 2,
                participant_id: 2,
                side: Side::Bid,
                price: 100,
                quantity: 4,
            })
            .expect("partial fill should succeed");

        assert_eq!(
            result.executions.as_slice(),
            &[Execution::new(1, 2, 100, 4)]
        );
        assert_eq!(book.best_ask(), Some((100, 6)));
    }

    #[test]
    fn self_trade_is_rejected_and_existing_order_remains() {
        let mut book = OrderBook::default();
        let _ = book.submit_limit(NewOrderRequest {
            id: 1,
            participant_id: 77,
            side: Side::Ask,
            price: 100,
            quantity: 5,
        });

        let error = book
            .submit_limit(NewOrderRequest {
                id: 2,
                participant_id: 77,
                side: Side::Bid,
                price: 101,
                quantity: 5,
            })
            .expect_err("self trade must fail");

        assert_eq!(
            error,
            OrderBookError::SelfTrade {
                resting_order_id: 1,
                incoming_order_id: 2,
            }
        );
        assert_eq!(book.best_ask(), Some((100, 5)));
        assert_eq!(book.total_resting_orders(), 1);
    }

    #[test]
    fn replay_rebuilds_book_from_state_events() {
        let events = [
            Event::Accepted(Order {
                id: 1,
                participant_id: 1,
                side: Side::Ask,
                price: 100,
                quantity: 10,
            }),
            Event::Executed(Execution::new(1, 2, 100, 4)),
            Event::Cancelled(crate::CancelledOrder {
                id: 1,
                participant_id: 1,
                side: Side::Ask,
                price: 100,
                quantity: 6,
            }),
        ];

        let replayed = OrderBook::replay(events).expect("replay should succeed");
        assert_eq!(replayed.best_ask(), None);
        assert_eq!(replayed.best_bid(), None);
        assert_eq!(replayed.total_resting_orders(), 0);
        assert_eq!(replayed.event_log().len(), 3);
    }

    #[test]
    fn concurrent_snapshot_reads_latest_published_state() {
        let book = ConcurrentOrderBook::default();
        let _ = book
            .submit_limit(NewOrderRequest {
                id: 1,
                participant_id: 1,
                side: Side::Bid,
                price: 123,
                quantity: 9,
            })
            .expect("limit order should succeed");

        let snapshot = book.snapshot();
        assert_eq!(snapshot.best_bid(), Some((123, 9)));
    }

    #[test]
    fn event_types_are_serializable() {
        let event = Event::Accepted(Order {
            id: 7,
            participant_id: 3,
            side: Side::Bid,
            price: 150,
            quantity: 2,
        });
        let json = serde_json::to_string(&event).expect("event should serialize");

        assert!(json.contains("Accepted"));
    }
}
