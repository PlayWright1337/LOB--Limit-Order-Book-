use std::collections::{BTreeMap, HashMap, VecDeque};

use crate::types::{
    CancelledOrder, Event, Execution, ExecutionVec, MarketOrderRequest, NewOrderRequest, Order,
    OrderBookError, OrderId, Price, Quantity, Side, SubmissionOutcome,
};

#[derive(Debug, Clone, Default)]
pub struct OrderBook {
    bids: BTreeMap<Price, PriceLevel>,
    asks: BTreeMap<Price, PriceLevel>,
    order_index: HashMap<OrderId, OrderLocator>,
    event_log: Vec<Event>,
    live_orders: usize,
}

#[derive(Debug, Clone, Default)]
pub struct PriceLevel {
    orders: VecDeque<Order>,
    total_volume: Quantity,
    head_offset: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct OrderLocator {
    side: Side,
    price: Price,
    offset: usize,
}

impl OrderBook {
    /// Adds a limit order, immediately matches crossed liquidity, and rests any remainder.
    pub fn submit_limit(
        &mut self,
        request: NewOrderRequest,
    ) -> Result<SubmissionOutcome, OrderBookError> {
        self.validate_request(request.id, request.quantity)?;

        let mut remaining = request.quantity;
        let mut executions = ExecutionVec::new();

        while remaining > 0 {
            let Some(best_price) = self.best_price_for(request.side.opposite()) else {
                break;
            };
            if !Self::price_crosses(request.side, request.price, best_price) {
                break;
            }

            let resting = match self.peek_front(request.side.opposite(), best_price) {
                Some(order) => order,
                None => {
                    self.remove_level_if_empty(request.side.opposite(), best_price);
                    continue;
                }
            };

            if resting.participant_id == request.participant_id {
                return Err(OrderBookError::SelfTrade {
                    resting_order_id: resting.id,
                    incoming_order_id: request.id,
                });
            }

            let traded = remaining.min(resting.quantity);
            let execution = Execution::new(resting.id, request.id, best_price, traded);
            executions.push(execution);
            self.apply_execution(execution)?;
            self.event_log.push(Event::Executed(execution));
            remaining -= traded;
        }

        let mut resting_order_id = None;
        if remaining > 0 {
            let order = Order {
                id: request.id,
                participant_id: request.participant_id,
                side: request.side,
                price: request.price,
                quantity: remaining,
            };
            self.insert_resting_order(order);
            self.event_log.push(Event::Accepted(order));
            resting_order_id = Some(request.id);
        }

        Ok(SubmissionOutcome {
            executions,
            resting_order_id,
            unfilled_quantity: remaining,
        })
    }

    /// Executes a market order against the best available liquidity until the requested size is filled or the book is empty.
    pub fn submit_market(
        &mut self,
        request: MarketOrderRequest,
    ) -> Result<SubmissionOutcome, OrderBookError> {
        self.validate_request(request.id, request.quantity)?;

        let mut remaining = request.quantity;
        let mut executions = ExecutionVec::new();

        while remaining > 0 {
            let Some(best_price) = self.best_price_for(request.side.opposite()) else {
                break;
            };
            let resting = match self.peek_front(request.side.opposite(), best_price) {
                Some(order) => order,
                None => {
                    self.remove_level_if_empty(request.side.opposite(), best_price);
                    continue;
                }
            };

            if resting.participant_id == request.participant_id {
                return Err(OrderBookError::SelfTrade {
                    resting_order_id: resting.id,
                    incoming_order_id: request.id,
                });
            }

            let traded = remaining.min(resting.quantity);
            let execution = Execution::new(resting.id, request.id, best_price, traded);
            executions.push(execution);
            self.apply_execution(execution)?;
            self.event_log.push(Event::Executed(execution));
            remaining -= traded;
        }

        Ok(SubmissionOutcome {
            executions,
            resting_order_id: None,
            unfilled_quantity: remaining,
        })
    }

    /// Cancels a live order by id in O(1) via the locator index and lazy tombstoning within the price level.
    pub fn cancel(&mut self, order_id: OrderId) -> Result<CancelledOrder, OrderBookError> {
        let locator = self
            .order_index
            .remove(&order_id)
            .ok_or(OrderBookError::UnknownOrder(order_id))?;
        let cancelled = {
            let level = self
                .book_side_mut(locator.side)
                .get_mut(&locator.price)
                .ok_or(OrderBookError::ReplayInvariantBroken)?;

            let index = locator
                .offset
                .checked_sub(level.head_offset)
                .ok_or(OrderBookError::ReplayInvariantBroken)?;
            let Some(order) = level.orders.get_mut(index) else {
                return Err(OrderBookError::ReplayInvariantBroken);
            };
            if order.id != order_id || order.quantity == 0 {
                return Err(OrderBookError::ReplayInvariantBroken);
            }

            let cancelled = CancelledOrder {
                id: order.id,
                participant_id: order.participant_id,
                side: order.side,
                price: order.price,
                quantity: order.quantity,
            };
            level.total_volume = level
                .total_volume
                .checked_sub(order.quantity)
                .ok_or(OrderBookError::ReplayInvariantBroken)?;
            order.quantity = 0;
            Self::compact_front(level);
            cancelled
        };
        self.live_orders = self
            .live_orders
            .checked_sub(1)
            .ok_or(OrderBookError::ReplayInvariantBroken)?;
        self.remove_level_if_empty(locator.side, locator.price);
        self.event_log.push(Event::Cancelled(cancelled));
        Ok(cancelled)
    }

    /// Rebuilds the order book from a deterministic stream of state events.
    pub fn replay(events: impl IntoIterator<Item = Event>) -> Result<Self, OrderBookError> {
        let mut book = Self::default();
        for event in events {
            book.apply(event)?;
            book.event_log.push(event);
        }
        Ok(book)
    }

    /// Applies a previously recorded state event without running matching logic.
    pub fn apply(&mut self, event: Event) -> Result<(), OrderBookError> {
        match event {
            Event::Accepted(order) => {
                self.validate_request(order.id, order.quantity)?;
                self.insert_resting_order(order);
                Ok(())
            }
            Event::Executed(execution) => self.apply_execution(execution),
            Event::Cancelled(cancelled) => self.apply_cancel(cancelled),
        }
    }

    /// Returns the best bid and its aggregated level volume.
    pub fn best_bid(&self) -> Option<(Price, Quantity)> {
        self.bids
            .last_key_value()
            .map(|(price, level)| (*price, level.total_volume))
    }

    /// Returns the best ask and its aggregated level volume.
    pub fn best_ask(&self) -> Option<(Price, Quantity)> {
        self.asks
            .first_key_value()
            .map(|(price, level)| (*price, level.total_volume))
    }

    /// Returns the number of currently live resting orders.
    pub fn total_resting_orders(&self) -> usize {
        self.live_orders
    }

    /// Returns the event-sourced state transition log.
    pub fn event_log(&self) -> &[Event] {
        &self.event_log
    }

    fn validate_request(
        &self,
        order_id: OrderId,
        quantity: Quantity,
    ) -> Result<(), OrderBookError> {
        if quantity == 0 {
            return Err(OrderBookError::InvalidQuantity);
        }
        if self.order_index.contains_key(&order_id) {
            return Err(OrderBookError::DuplicateOrderId(order_id));
        }
        Ok(())
    }

    fn insert_resting_order(&mut self, order: Order) {
        let level = self
            .book_side_mut(order.side)
            .entry(order.price)
            .or_default();
        let offset = level.head_offset + level.orders.len();
        level.total_volume += order.quantity;
        level.orders.push_back(order);
        self.order_index.insert(
            order.id,
            OrderLocator {
                side: order.side,
                price: order.price,
                offset,
            },
        );
        self.live_orders += 1;
    }

    fn apply_execution(&mut self, execution: Execution) -> Result<(), OrderBookError> {
        let locator = self
            .order_index
            .get(&execution.maker_order_id)
            .copied()
            .ok_or(OrderBookError::ReplayInvariantBroken)?;
        let mut completed_order_id = None;
        {
            let level = self
                .book_side_mut(locator.side)
                .get_mut(&locator.price)
                .ok_or(OrderBookError::ReplayInvariantBroken)?;
            let index = locator
                .offset
                .checked_sub(level.head_offset)
                .ok_or(OrderBookError::ReplayInvariantBroken)?;
            let Some(order) = level.orders.get_mut(index) else {
                return Err(OrderBookError::ReplayInvariantBroken);
            };
            if order.id != execution.maker_order_id
                || order.price != execution.price
                || execution.quantity > order.quantity
            {
                return Err(OrderBookError::ReplayInvariantBroken);
            }

            order.quantity -= execution.quantity;
            level.total_volume = level
                .total_volume
                .checked_sub(execution.quantity)
                .ok_or(OrderBookError::ReplayInvariantBroken)?;
            if order.quantity == 0 {
                completed_order_id = Some(order.id);
            }
            Self::compact_front(level);
        }
        if let Some(order_id) = completed_order_id {
            self.order_index.remove(&order_id);
            self.live_orders = self
                .live_orders
                .checked_sub(1)
                .ok_or(OrderBookError::ReplayInvariantBroken)?;
        }
        self.remove_level_if_empty(locator.side, locator.price);
        Ok(())
    }

    fn apply_cancel(&mut self, cancelled: CancelledOrder) -> Result<(), OrderBookError> {
        let locator = self
            .order_index
            .remove(&cancelled.id)
            .ok_or(OrderBookError::ReplayInvariantBroken)?;
        {
            let level = self
                .book_side_mut(locator.side)
                .get_mut(&locator.price)
                .ok_or(OrderBookError::ReplayInvariantBroken)?;
            let index = locator
                .offset
                .checked_sub(level.head_offset)
                .ok_or(OrderBookError::ReplayInvariantBroken)?;
            let Some(order) = level.orders.get_mut(index) else {
                return Err(OrderBookError::ReplayInvariantBroken);
            };
            if order.id != cancelled.id || order.quantity != cancelled.quantity {
                return Err(OrderBookError::ReplayInvariantBroken);
            }

            level.total_volume = level
                .total_volume
                .checked_sub(order.quantity)
                .ok_or(OrderBookError::ReplayInvariantBroken)?;
            order.quantity = 0;
            Self::compact_front(level);
        }
        self.live_orders = self
            .live_orders
            .checked_sub(1)
            .ok_or(OrderBookError::ReplayInvariantBroken)?;
        self.remove_level_if_empty(locator.side, locator.price);
        Ok(())
    }

    fn compact_front(level: &mut PriceLevel) {
        while level
            .orders
            .front()
            .is_some_and(|order| order.quantity == 0)
        {
            let _ = level.orders.pop_front();
            level.head_offset += 1;
        }
    }

    fn peek_front(&mut self, side: Side, price: Price) -> Option<Order> {
        let level = self.book_side_mut(side).get_mut(&price)?;
        Self::compact_front(level);
        level.orders.front().copied()
    }

    fn best_price_for(&self, side: Side) -> Option<Price> {
        match side {
            Side::Bid => self.bids.last_key_value().map(|(price, _)| *price),
            Side::Ask => self.asks.first_key_value().map(|(price, _)| *price),
        }
    }

    fn remove_level_if_empty(&mut self, side: Side, price: Price) {
        let should_remove = self
            .book_side(side)
            .get(&price)
            .is_some_and(|level| level.total_volume == 0);
        if should_remove {
            self.book_side_mut(side).remove(&price);
        }
    }

    fn price_crosses(side: Side, limit_price: Price, book_price: Price) -> bool {
        match side {
            Side::Bid => book_price <= limit_price,
            Side::Ask => book_price >= limit_price,
        }
    }

    fn book_side(&self, side: Side) -> &BTreeMap<Price, PriceLevel> {
        match side {
            Side::Bid => &self.bids,
            Side::Ask => &self.asks,
        }
    }

    fn book_side_mut(&mut self, side: Side) -> &mut BTreeMap<Price, PriceLevel> {
        match side {
            Side::Bid => &mut self.bids,
            Side::Ask => &mut self.asks,
        }
    }
}
