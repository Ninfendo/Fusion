//! One order's life, driven by local requests and FIX execution reports.
//!
//! An [`Order`] is a pure state machine. [`Order::apply`] takes an
//! [`OrderEvent`] and either changes the order or returns an [`OrderError`]
//! and leaves the order exactly as it was. It does no I/O and reads no clock,
//! so replaying the same events always rebuilds the same order.
//!
//! The fills are the truth for the filled quantity. Every execution report's
//! CumQty and LeavesQty are checked against them, so a missed, doubled or
//! out-of-order report shows up as an error instead of a silent wrong position.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use crate::fix::{ExecType, OrdStatus, Side};
use crate::ids::{ClOrdId, ExecId, InstrumentId, OrderId};
use crate::num::{Decimal, Price, Quantity};
use crate::time::Timestamp;

/// What the trader asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewOrder {
    /// Fusion's id. It never changes.
    pub id: OrderId,
    /// ClOrdID (11) of the new-order request.
    pub cl_ord_id: ClOrdId,
    /// The instrument.
    pub instrument: InstrumentId,
    /// Buy or sell.
    pub side: Side,
    /// OrderQty (38).
    pub quantity: Quantity,
    /// Price (44). `None` for a market order.
    pub limit_price: Option<Price>,
}

/// A FIX execution report (MsgType 8), already parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionReport {
    /// ExecID (17).
    pub exec_id: ExecId,
    /// ExecType (150).
    pub exec_type: ExecType,
    /// OrdStatus (39).
    pub ord_status: OrdStatus,
    /// ClOrdID (11).
    pub cl_ord_id: ClOrdId,
    /// OrigClOrdID (41).
    pub orig_cl_ord_id: Option<ClOrdId>,
    /// ExecRefID (19): the trade that a correction or a bust refers to.
    pub exec_ref_id: Option<ExecId>,
    /// OrderQty (38), when the venue reports it.
    pub order_qty: Option<Quantity>,
    /// Price (44), when the venue reports it.
    pub price: Option<Price>,
    /// CumQty (14).
    pub cum_qty: Quantity,
    /// LeavesQty (151).
    pub leaves_qty: Quantity,
    /// LastQty (32), on trades and trade corrections.
    pub last_qty: Option<Quantity>,
    /// LastPx (31), on trades and trade corrections.
    pub last_px: Option<Price>,
    /// TransactTime (60).
    pub transact_time: Timestamp,
}

/// Everything that can happen to an order. The journal stores these, and
/// [`Order::replay`] rebuilds an order from them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrderEvent {
    /// The order was sent (FIX NewOrderSingle, MsgType D).
    Submitted(NewOrder),
    /// A cancel was sent (FIX OrderCancelRequest, MsgType F).
    CancelRequested {
        /// ClOrdID (11) of the cancel request.
        cl_ord_id: ClOrdId,
    },
    /// A replace was sent (FIX OrderCancelReplaceRequest, MsgType G).
    ReplaceRequested {
        /// ClOrdID (11) of the replace request.
        cl_ord_id: ClOrdId,
        /// The new OrderQty.
        quantity: Quantity,
        /// The new limit price.
        limit_price: Option<Price>,
    },
    /// The venue rejected a cancel or a replace (FIX OrderCancelReject,
    /// MsgType 9).
    RequestRejected {
        /// ClOrdID (11) of the rejected request.
        cl_ord_id: ClOrdId,
    },
    /// An execution report arrived.
    Execution(Box<ExecutionReport>),
}

/// What a pending request asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestKind {
    /// Cancel the order.
    Cancel,
    /// Replace quantity and price.
    Replace {
        /// The new OrderQty.
        quantity: Quantity,
        /// The new limit price.
        limit_price: Option<Price>,
    },
}

/// A cancel or a replace the venue has not answered yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingRequest {
    /// What was asked.
    pub kind: RequestKind,
    /// ClOrdID (11) of the request.
    pub cl_ord_id: ClOrdId,
    /// The status to go back to if the venue rejects the request. Fills that
    /// arrive meanwhile update it.
    pub resume_status: OrdStatus,
}

/// One fill.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fill {
    /// LastQty (32).
    pub quantity: Quantity,
    /// LastPx (31).
    pub price: Price,
    /// TransactTime (60).
    pub at: Timestamp,
}

/// What [`Order::apply`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Applied {
    /// The order changed.
    Changed,
    /// The execution report was seen before (same ExecID), so it was ignored.
    Duplicate,
}

/// The quantity field that did not match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuantityField {
    /// CumQty (14).
    CumQty,
    /// LeavesQty (151).
    LeavesQty,
}

/// Why an event was refused. The order is unchanged after any error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrderError {
    /// A replay did not start with [`OrderEvent::Submitted`].
    NotSubmitted,
    /// [`OrderEvent::Submitted`] came for an order that already exists.
    AlreadySubmitted,
    /// The order is Filled, Canceled, Rejected or Expired.
    Terminal {
        /// The order's status.
        status: OrdStatus,
    },
    /// A cancel or replace is already waiting for an answer.
    RequestInFlight {
        /// ClOrdID of the waiting request.
        pending: ClOrdId,
    },
    /// A reject arrived for a request this order is not waiting for.
    NoMatchingRequest {
        /// ClOrdID in the reject.
        cl_ord_id: ClOrdId,
    },
    /// The report's OrdStatus cannot go with its ExecType.
    StatusDoesNotFit {
        /// ExecType (150).
        exec_type: ExecType,
        /// OrdStatus (39).
        ord_status: OrdStatus,
    },
    /// The report does not fit the order's state, for example a second New.
    UnexpectedExecType {
        /// ExecType (150).
        exec_type: ExecType,
        /// The order's status.
        status: OrdStatus,
    },
    /// A field the report needs is missing or not above zero.
    InvalidField {
        /// ExecType (150).
        exec_type: ExecType,
        /// The FIX field.
        field: &'static str,
    },
    /// A correction or a bust refers to a fill this order does not have.
    UnknownFill {
        /// ExecRefID (19).
        exec_ref_id: ExecId,
    },
    /// More would be filled than the order quantity.
    Overfill {
        /// The filled quantity.
        cum_qty: Quantity,
        /// The order quantity.
        order_qty: Quantity,
    },
    /// The venue's quantity differs from the fills this order knows about.
    QuantityMismatch {
        /// Which field.
        field: QuantityField,
        /// What the fills say.
        ours: Quantity,
        /// What the venue says.
        reported: Quantity,
    },
}

impl fmt::Display for OrderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotSubmitted => f.write_str("the first event of an order must be Submitted"),
            Self::AlreadySubmitted => f.write_str("the order was already submitted"),
            Self::Terminal { status } => write!(f, "the order is {status} and cannot change"),
            Self::RequestInFlight { pending } => {
                write!(f, "request {pending} is still waiting for an answer")
            }
            Self::NoMatchingRequest { cl_ord_id } => {
                write!(f, "no request {cl_ord_id} is waiting for an answer")
            }
            Self::StatusDoesNotFit {
                exec_type,
                ord_status,
            } => write!(
                f,
                "OrdStatus {ord_status} cannot go with ExecType {exec_type}"
            ),
            Self::UnexpectedExecType { exec_type, status } => {
                write!(
                    f,
                    "ExecType {exec_type} does not fit an order that is {status}"
                )
            }
            Self::InvalidField { exec_type, field } => {
                write!(f, "ExecType {exec_type} needs {field} above zero")
            }
            Self::UnknownFill { exec_ref_id } => write!(f, "no fill with ExecID {exec_ref_id}"),
            Self::Overfill { cum_qty, order_qty } => {
                write!(
                    f,
                    "filled {cum_qty} would exceed the order quantity {order_qty}"
                )
            }
            Self::QuantityMismatch {
                field,
                ours,
                reported,
            } => write!(
                f,
                "{field:?} is {ours} by our fills but {reported} by the venue"
            ),
        }
    }
}

impl std::error::Error for OrderError {}

/// One order and everything that has happened to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Order {
    id: OrderId,
    instrument: InstrumentId,
    side: Side,
    quantity: Quantity,
    limit_price: Option<Price>,
    status: OrdStatus,
    cl_ord_id: ClOrdId,
    cl_ord_ids: Vec<ClOrdId>,
    pending: Option<PendingRequest>,
    fills: BTreeMap<ExecId, Fill>,
    seen: BTreeSet<ExecId>,
}

impl Order {
    /// A newly sent order, waiting for the venue: PendingNew.
    pub fn submit(order: NewOrder) -> Self {
        Self {
            id: order.id,
            instrument: order.instrument,
            side: order.side,
            quantity: order.quantity,
            limit_price: order.limit_price,
            status: OrdStatus::PendingNew,
            cl_ord_ids: vec![order.cl_ord_id.clone()],
            cl_ord_id: order.cl_ord_id,
            pending: None,
            fills: BTreeMap::new(),
            seen: BTreeSet::new(),
        }
    }

    /// Rebuilds an order from its events. The first must be `Submitted`.
    pub fn replay<'a>(
        events: impl IntoIterator<Item = &'a OrderEvent>,
    ) -> Result<Self, OrderError> {
        let mut events = events.into_iter();
        let mut order = match events.next() {
            Some(OrderEvent::Submitted(new)) => Self::submit(new.clone()),
            _ => return Err(OrderError::NotSubmitted),
        };
        for event in events {
            order.apply(event)?;
        }
        Ok(order)
    }

    /// Fusion's id.
    pub fn id(&self) -> OrderId {
        self.id
    }

    /// The instrument.
    pub fn instrument(&self) -> InstrumentId {
        self.instrument
    }

    /// Buy or sell.
    pub fn side(&self) -> Side {
        self.side
    }

    /// OrderQty (38), after any replace.
    pub fn quantity(&self) -> Quantity {
        self.quantity
    }

    /// The limit price, after any replace. `None` for a market order.
    pub fn limit_price(&self) -> Option<Price> {
        self.limit_price
    }

    /// The order's status. While a cancel or replace waits for an answer it
    /// is PendingCancel or PendingReplace.
    pub fn status(&self) -> OrdStatus {
        self.status
    }

    /// True if the order can never trade again.
    pub fn is_terminal(&self) -> bool {
        self.status.is_terminal()
    }

    /// The ClOrdID the venue knows the order by now.
    pub fn cl_ord_id(&self) -> &ClOrdId {
        &self.cl_ord_id
    }

    /// Every ClOrdID this order has used, oldest first.
    pub fn cl_ord_ids(&self) -> &[ClOrdId] {
        &self.cl_ord_ids
    }

    /// The cancel or replace waiting for an answer, if any.
    pub fn pending(&self) -> Option<&PendingRequest> {
        self.pending.as_ref()
    }

    /// The fills, by ExecID.
    pub fn fills(&self) -> &BTreeMap<ExecId, Fill> {
        &self.fills
    }

    /// The filled quantity: the sum of the fills.
    pub fn cum_qty(&self) -> Quantity {
        self.fills
            .values()
            .fold(Quantity::ZERO, |sum, fill| sum + fill.quantity)
    }

    /// The quantity still working at the venue.
    pub fn leaves_qty(&self) -> Quantity {
        if self.status.holds_leaves() {
            self.quantity - self.cum_qty()
        } else {
            Quantity::ZERO
        }
    }

    /// The average fill price, or `None` before the first fill.
    pub fn avg_price(&self) -> Option<Decimal> {
        let cum = self.cum_qty();
        if cum.is_zero() {
            return None;
        }
        let value: Decimal = self
            .fills
            .values()
            .map(|fill| fill.price.value() * fill.quantity.value())
            .sum();
        Some(value / cum.value())
    }

    /// Applies one event. On error the order is unchanged.
    ///
    /// The checks run on a copy of the order. That costs one clone per event,
    /// which is fine at order rates; market data never goes through here.
    pub fn apply(&mut self, event: &OrderEvent) -> Result<Applied, OrderError> {
        let mut next = self.clone();
        let applied = next.apply_in_place(event)?;
        *self = next;
        Ok(applied)
    }

    fn apply_in_place(&mut self, event: &OrderEvent) -> Result<Applied, OrderError> {
        match event {
            OrderEvent::Submitted(_) => Err(OrderError::AlreadySubmitted),
            OrderEvent::CancelRequested { cl_ord_id } => {
                self.request(cl_ord_id, RequestKind::Cancel)
            }
            OrderEvent::ReplaceRequested {
                cl_ord_id,
                quantity,
                limit_price,
            } => self.request(
                cl_ord_id,
                RequestKind::Replace {
                    quantity: *quantity,
                    limit_price: *limit_price,
                },
            ),
            OrderEvent::RequestRejected { cl_ord_id } => self.request_rejected(cl_ord_id),
            OrderEvent::Execution(report) => self.execution(report),
        }
    }

    fn request(&mut self, cl_ord_id: &ClOrdId, kind: RequestKind) -> Result<Applied, OrderError> {
        if self.status.is_terminal() {
            return Err(OrderError::Terminal {
                status: self.status,
            });
        }
        if let Some(pending) = &self.pending {
            return Err(OrderError::RequestInFlight {
                pending: pending.cl_ord_id.clone(),
            });
        }
        if let RequestKind::Replace { quantity, .. } = kind {
            let cum_qty = self.cum_qty();
            if quantity < cum_qty || !quantity.is_positive() {
                return Err(OrderError::Overfill {
                    cum_qty,
                    order_qty: quantity,
                });
            }
        }
        self.pending = Some(PendingRequest {
            kind,
            cl_ord_id: cl_ord_id.clone(),
            resume_status: self.status,
        });
        self.cl_ord_ids.push(cl_ord_id.clone());
        self.status = match kind {
            RequestKind::Cancel => OrdStatus::PendingCancel,
            RequestKind::Replace { .. } => OrdStatus::PendingReplace,
        };
        Ok(Applied::Changed)
    }

    fn request_rejected(&mut self, cl_ord_id: &ClOrdId) -> Result<Applied, OrderError> {
        match self.pending.take() {
            Some(pending) if &pending.cl_ord_id == cl_ord_id => {
                self.status = pending.resume_status;
                Ok(Applied::Changed)
            }
            _ => Err(OrderError::NoMatchingRequest {
                cl_ord_id: cl_ord_id.clone(),
            }),
        }
    }

    fn execution(&mut self, report: &ExecutionReport) -> Result<Applied, OrderError> {
        // Status reports may reuse an ExecID (often "0"), so they are not deduplicated.
        let deduplicate = report.exec_type != ExecType::OrderStatus;
        if deduplicate && self.seen.contains(&report.exec_id) {
            return Ok(Applied::Duplicate);
        }
        self.check_report_fits(report)?;
        self.apply_exec_type(report)?;
        self.set_reported_status(report.ord_status);
        self.check_quantities(report)?;
        if deduplicate {
            self.seen.insert(report.exec_id.clone());
        }
        Ok(Applied::Changed)
    }

    /// Refuses a report whose status does not fit its exec type, and any
    /// report but a correction on a terminal order.
    fn check_report_fits(&self, report: &ExecutionReport) -> Result<(), OrderError> {
        if !report.exec_type.allows_status(report.ord_status) {
            return Err(OrderError::StatusDoesNotFit {
                exec_type: report.exec_type,
                ord_status: report.ord_status,
            });
        }
        let corrective = matches!(
            report.exec_type,
            ExecType::TradeCorrect
                | ExecType::TradeCancel
                | ExecType::Restated
                | ExecType::OrderStatus
        );
        if self.status.is_terminal() && !corrective {
            return Err(OrderError::Terminal {
                status: self.status,
            });
        }
        Ok(())
    }

    /// Applies what the exec type itself changes: fills, quantities, ids.
    /// Status-only exec types change nothing here.
    fn apply_exec_type(&mut self, report: &ExecutionReport) -> Result<(), OrderError> {
        match report.exec_type {
            // A fill moves the order out of PendingNew, so an order that is
            // still unacknowledged has no fills and a reject is safe.
            ExecType::PendingNew | ExecType::New | ExecType::Rejected => {
                self.expect_unacknowledged(report)?;
            }
            ExecType::Trade => self.add_fill(report)?,
            ExecType::TradeCorrect => self.correct_fill(report)?,
            ExecType::TradeCancel => self.bust_fill(report)?,
            ExecType::Replaced => self.replaced(report)?,
            ExecType::Restated => self.restated(report)?,
            ExecType::Canceled
            | ExecType::Expired
            | ExecType::PendingCancel
            | ExecType::PendingReplace
            | ExecType::DoneForDay
            | ExecType::Stopped
            | ExecType::Suspended
            | ExecType::Calculated
            | ExecType::OrderStatus => {}
        }
        Ok(())
    }

    /// True while the venue has not acknowledged the order yet.
    fn awaiting_ack(&self) -> bool {
        self.status == OrdStatus::PendingNew
            || self
                .pending
                .as_ref()
                .is_some_and(|p| p.resume_status == OrdStatus::PendingNew)
    }

    fn expect_unacknowledged(&self, report: &ExecutionReport) -> Result<(), OrderError> {
        if self.awaiting_ack() {
            Ok(())
        } else {
            Err(OrderError::UnexpectedExecType {
                exec_type: report.exec_type,
                status: self.status,
            })
        }
    }

    fn trade_fields(report: &ExecutionReport) -> Result<Fill, OrderError> {
        let invalid = |field| OrderError::InvalidField {
            exec_type: report.exec_type,
            field,
        };
        let quantity = report
            .last_qty
            .filter(|q| q.is_positive())
            .ok_or(invalid("LastQty (32)"))?;
        let price = report.last_px.ok_or(invalid("LastPx (31)"))?;
        Ok(Fill {
            quantity,
            price,
            at: report.transact_time,
        })
    }

    fn fill_reference(report: &ExecutionReport) -> Result<&ExecId, OrderError> {
        report.exec_ref_id.as_ref().ok_or(OrderError::InvalidField {
            exec_type: report.exec_type,
            field: "ExecRefID (19)",
        })
    }

    fn check_overfill(&self) -> Result<(), OrderError> {
        let cum_qty = self.cum_qty();
        if cum_qty > self.quantity {
            return Err(OrderError::Overfill {
                cum_qty,
                order_qty: self.quantity,
            });
        }
        Ok(())
    }

    fn add_fill(&mut self, report: &ExecutionReport) -> Result<(), OrderError> {
        let fill = Self::trade_fields(report)?;
        self.fills.insert(report.exec_id.clone(), fill);
        self.check_overfill()
    }

    fn correct_fill(&mut self, report: &ExecutionReport) -> Result<(), OrderError> {
        let reference = Self::fill_reference(report)?;
        let corrected = Self::trade_fields(report)?;
        let fill = self
            .fills
            .get_mut(reference)
            .ok_or_else(|| OrderError::UnknownFill {
                exec_ref_id: reference.clone(),
            })?;
        *fill = corrected;
        self.check_overfill()
    }

    fn bust_fill(&mut self, report: &ExecutionReport) -> Result<(), OrderError> {
        let reference = Self::fill_reference(report)?;
        self.fills
            .remove(reference)
            .map(|_| ())
            .ok_or_else(|| OrderError::UnknownFill {
                exec_ref_id: reference.clone(),
            })
    }

    fn replaced(&mut self, report: &ExecutionReport) -> Result<(), OrderError> {
        let requested = match self.pending.as_ref().map(|p| p.kind) {
            Some(RequestKind::Replace {
                quantity,
                limit_price,
            }) => {
                self.pending = None;
                Some((quantity, limit_price))
            }
            _ => None,
        };
        if let Some(quantity) = report.order_qty.or(requested.map(|(q, _)| q)) {
            self.quantity = quantity;
        }
        if let Some(price) = report.price.or(requested.and_then(|(_, p)| p)) {
            self.limit_price = Some(price);
        }
        self.cl_ord_id = report.cl_ord_id.clone();
        if !self.cl_ord_ids.contains(&report.cl_ord_id) {
            self.cl_ord_ids.push(report.cl_ord_id.clone());
        }
        self.check_overfill()
    }

    fn restated(&mut self, report: &ExecutionReport) -> Result<(), OrderError> {
        if let Some(quantity) = report.order_qty {
            self.quantity = quantity;
        }
        if let Some(price) = report.price {
            self.limit_price = Some(price);
        }
        self.check_overfill()
    }

    /// Takes the venue's status, but keeps a local PendingCancel or
    /// PendingReplace while that request waits for an answer.
    fn set_reported_status(&mut self, reported: OrdStatus) {
        if reported.is_terminal() {
            self.pending = None;
            self.status = reported;
            return;
        }
        match &mut self.pending {
            Some(pending) if reported.is_working() => pending.resume_status = reported,
            _ => self.status = reported,
        }
    }

    fn check_quantities(&self, report: &ExecutionReport) -> Result<(), OrderError> {
        let cum_qty = self.cum_qty();
        if report.cum_qty != cum_qty {
            return Err(OrderError::QuantityMismatch {
                field: QuantityField::CumQty,
                ours: cum_qty,
                reported: report.cum_qty,
            });
        }
        if report.ord_status == OrdStatus::Filled && cum_qty != self.quantity {
            return Err(OrderError::QuantityMismatch {
                field: QuantityField::CumQty,
                ours: cum_qty,
                reported: self.quantity,
            });
        }
        let expected_leaves = if report.ord_status.holds_leaves() {
            Some(self.quantity - cum_qty)
        } else if report.ord_status.is_terminal() {
            Some(Quantity::ZERO)
        } else {
            // DoneForDay and Calculated: FIX lets the venue report either.
            None
        };
        if let Some(leaves) = expected_leaves
            && report.leaves_qty != leaves
        {
            return Err(OrderError::QuantityMismatch {
                field: QuantityField::LeavesQty,
                ours: leaves,
                reported: report.leaves_qty,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
