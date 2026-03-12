use smallvec::SmallVec;

pub type Price = u64;
pub type Quantity = u64;
pub type OrderId = u64;
pub type ParticipantId = u32;
pub type ExecutionVec = SmallVec<[Execution; 4]>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Side {
    Bid,
    Ask,
}

impl Side {
    pub const fn opposite(self) -> Self {
        match self {
            Self::Bid => Self::Ask,
            Self::Ask => Self::Bid,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Order {
    pub id: OrderId,
    pub participant_id: ParticipantId,
    pub side: Side,
    pub price: Price,
    pub quantity: Quantity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct NewOrderRequest {
    pub id: OrderId,
    pub participant_id: ParticipantId,
    pub side: Side,
    pub price: Price,
    pub quantity: Quantity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MarketOrderRequest {
    pub id: OrderId,
    pub participant_id: ParticipantId,
    pub side: Side,
    pub quantity: Quantity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Execution {
    pub maker_order_id: OrderId,
    pub taker_order_id: OrderId,
    pub price: Price,
    pub quantity: Quantity,
}

impl Execution {
    pub const fn new(
        maker_order_id: OrderId,
        taker_order_id: OrderId,
        price: Price,
        quantity: Quantity,
    ) -> Self {
        Self {
            maker_order_id,
            taker_order_id,
            price,
            quantity,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CancelledOrder {
    pub id: OrderId,
    pub participant_id: ParticipantId,
    pub side: Side,
    pub price: Price,
    pub quantity: Quantity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Event {
    Accepted(Order),
    Executed(Execution),
    Cancelled(CancelledOrder),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubmissionOutcome {
    pub executions: ExecutionVec,
    pub resting_order_id: Option<OrderId>,
    pub unfilled_quantity: Quantity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderBookError {
    DuplicateOrderId(OrderId),
    InvalidQuantity,
    UnknownOrder(OrderId),
    SelfTrade {
        resting_order_id: OrderId,
        incoming_order_id: OrderId,
    },
    ReplayInvariantBroken,
}
