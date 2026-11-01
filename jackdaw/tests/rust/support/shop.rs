//! A small order domain whose tasks count their side effects, so tests can say how often something really ran.
#![allow(dead_code)]

use super::*;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Order {
    pub id: String,
    pub total_cents: u64,
    pub items: Vec<String>,
}

impl Order {
    pub fn new(id: &str, total_cents: u64) -> Self {
        Self {
            id: id.into(),
            total_cents,
            items: vec!["apple".into(), "pear".into(), "fig".into()],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Receipt {
    pub order_id: String,
    pub amount_cents: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Shipment {
    pub order_id: String,
    pub tracking: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Priced {
    pub item: String,
    pub cents: u64,
}

/// An event a workflow can wait for.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Approved {
    pub by: String,
}

#[derive(Default)]
pub struct Shop {
    pub charges: Counter,
    pub refunds: Counter,
    pub ships: Counter,
    pub audits: Counter,
    pub notices: Counter,
    /// When set, `charge` fails with this HTTP-style status instead of succeeding.
    pub charge_status: Option<u16>,
    pub in_flight: AtomicUsize,
    pub max_in_flight: AtomicUsize,
}

impl Shop {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// A shop whose payment gateway rejects every charge with `status`.
    pub fn failing_charges(status: u16) -> Arc<Self> {
        Arc::new(Self {
            charge_status: Some(status),
            ..Self::default()
        })
    }
}

#[tasks]
impl Shop {
    #[task]
    pub async fn charge(self: Arc<Self>, order: Order) -> Result<Receipt, TaskError> {
        self.charges.hit();
        match self.charge_status {
            Some(status) => Err(TaskError::fault(error_types::COMMUNICATION)
                .status(status)
                .detail("card declined")),
            None => Ok(Receipt {
                order_id: order.id,
                amount_cents: order.total_cents,
            }),
        }
    }

    #[task]
    pub async fn refund(self: Arc<Self>, _order: Order) -> Result<(), TaskError> {
        self.refunds.hit();
        Ok(())
    }

    #[task]
    pub async fn ship(self: Arc<Self>, receipt: Receipt) -> Result<Shipment, TaskError> {
        self.ships.hit();
        Ok(Shipment {
            order_id: receipt.order_id,
            tracking: "TRACK-1".into(),
        })
    }

    #[task]
    pub async fn audit(self: Arc<Self>, _order: Order) -> Result<(), TaskError> {
        self.audits.hit();
        Ok(())
    }

    #[task]
    pub async fn notify(self: Arc<Self>, _order: Order) -> Result<(), TaskError> {
        self.notices.hit();
        Ok(())
    }

    /// Takes one virtual second and records how many prices were in flight at once.
    #[task]
    pub async fn price(self: Arc<Self>, item: String) -> Result<Priced, TaskError> {
        let now = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_in_flight.fetch_max(now, Ordering::SeqCst);
        tokio::time::sleep(1.secs()).await;
        self.in_flight.fetch_sub(1, Ordering::SeqCst);
        Ok(Priced {
            cents: item.len() as u64 * 100,
            item,
        })
    }
}
