use std::sync::Arc;

use arc_swap::ArcSwap;
use parking_lot::Mutex;

use crate::book::OrderBook;
use crate::types::{
    CancelledOrder, MarketOrderRequest, NewOrderRequest, OrderBookError, OrderId, SubmissionOutcome,
};

#[derive(Debug)]
pub struct ConcurrentOrderBook {
    writer: Mutex<OrderBook>,
    readable: ArcSwap<OrderBook>,
}

impl Default for ConcurrentOrderBook {
    fn default() -> Self {
        let initial = Arc::new(OrderBook::default());
        Self {
            writer: Mutex::new(OrderBook::default()),
            readable: ArcSwap::from(initial),
        }
    }
}

impl ConcurrentOrderBook {
    /// Submits a limit order and publishes a new snapshot with double-buffered lock-free reads.
    pub fn submit_limit(
        &self,
        request: NewOrderRequest,
    ) -> Result<SubmissionOutcome, OrderBookError> {
        let mut writer = self.writer.lock();
        let result = writer.submit_limit(request)?;
        self.readable.store(Arc::new(writer.clone()));
        Ok(result)
    }

    /// Submits a market order and publishes a new snapshot with double-buffered lock-free reads.
    pub fn submit_market(
        &self,
        request: MarketOrderRequest,
    ) -> Result<SubmissionOutcome, OrderBookError> {
        let mut writer = self.writer.lock();
        let result = writer.submit_market(request)?;
        self.readable.store(Arc::new(writer.clone()));
        Ok(result)
    }

    /// Cancels a live order and publishes a new snapshot with double-buffered lock-free reads.
    pub fn cancel(&self, order_id: OrderId) -> Result<CancelledOrder, OrderBookError> {
        let mut writer = self.writer.lock();
        let result = writer.cancel(order_id)?;
        self.readable.store(Arc::new(writer.clone()));
        Ok(result)
    }

    /// Returns the current immutable snapshot without taking the writer lock.
    pub fn snapshot(&self) -> Arc<OrderBook> {
        self.readable.load_full()
    }
}
