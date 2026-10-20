//! The milestone 0 gate: every FIX status change of an order.

use super::*;

fn q(s: &str) -> Quantity {
    s.parse().expect("valid quantity")
}

fn p(s: &str) -> Price {
    s.parse().expect("valid price")
}

fn d(s: &str) -> Decimal {
    s.parse().expect("valid decimal")
}

fn cl(s: &str) -> ClOrdId {
    ClOrdId::new(s)
}

fn new_order() -> NewOrder {
    NewOrder {
        id: OrderId(1),
        cl_ord_id: cl("c1"),
        instrument: InstrumentId(7),
        side: Side::Buy,
        quantity: q("10"),
        limit_price: Some(p("100")),
    }
}

/// An execution report for `c1` with no trade fields.
fn report(
    exec_id: &str,
    exec_type: ExecType,
    ord_status: OrdStatus,
    cum: &str,
    leaves: &str,
) -> ExecutionReport {
    ExecutionReport {
        exec_id: ExecId::new(exec_id),
        exec_type,
        ord_status,
        cl_ord_id: cl("c1"),
        orig_cl_ord_id: None,
        exec_ref_id: None,
        order_qty: None,
        price: None,
        cum_qty: q(cum),
        leaves_qty: q(leaves),
        last_qty: None,
        last_px: None,
        transact_time: Timestamp::from_unix_nanos(1),
    }
}

fn trade(
    exec_id: &str,
    ord_status: OrdStatus,
    last: (&str, &str),
    cum: &str,
    leaves: &str,
) -> ExecutionReport {
    ExecutionReport {
        last_qty: Some(q(last.0)),
        last_px: Some(p(last.1)),
        ..report(exec_id, ExecType::Trade, ord_status, cum, leaves)
    }
}

fn exec(report: ExecutionReport) -> OrderEvent {
    OrderEvent::Execution(Box::new(report))
}

fn cancel(id: &str) -> OrderEvent {
    OrderEvent::CancelRequested { cl_ord_id: cl(id) }
}

fn replace(id: &str, quantity: &str, price: &str) -> OrderEvent {
    OrderEvent::ReplaceRequested {
        cl_ord_id: cl(id),
        quantity: q(quantity),
        limit_price: Some(p(price)),
    }
}

fn rejected(id: &str) -> OrderEvent {
    OrderEvent::RequestRejected { cl_ord_id: cl(id) }
}

fn apply(order: &mut Order, event: OrderEvent) -> Applied {
    order.apply(&event).expect("event should apply")
}

/// Refuses the event and checks the order did not change.
fn refuse(order: &mut Order, event: OrderEvent) -> OrderError {
    let before = order.clone();
    let err = order.apply(&event).expect_err("event should be refused");
    assert_eq!(*order, before, "a refused event changed the order");
    err
}

/// An order the venue has acknowledged: New, nothing filled.
fn acked() -> Order {
    let mut order = Order::submit(new_order());
    apply(
        &mut order,
        exec(report("e0", ExecType::New, OrdStatus::New, "0", "10")),
    );
    order
}

/// 4 of 10 filled at 100.
fn partially_filled() -> Order {
    let mut order = acked();
    apply(
        &mut order,
        exec(trade(
            "e1",
            OrdStatus::PartiallyFilled,
            ("4", "100"),
            "4",
            "6",
        )),
    );
    order
}

/// A cancelled order.
fn canceled() -> Order {
    let mut order = acked();
    apply(
        &mut order,
        exec(report(
            "e9",
            ExecType::Canceled,
            OrdStatus::Canceled,
            "0",
            "0",
        )),
    );
    order
}

// --- New order ---------------------------------------------------------------

#[test]
fn a_sent_order_is_pending_new() {
    let order = Order::submit(new_order());
    assert_eq!(order.status(), OrdStatus::PendingNew);
    assert_eq!(order.cum_qty(), Quantity::ZERO);
    assert_eq!(order.leaves_qty(), q("10"));
    assert_eq!(order.cl_ord_ids(), [cl("c1")]);
    assert_eq!(order.avg_price(), None);
    assert!(!order.is_terminal());
}

#[test]
fn the_venue_confirms_pending_new() {
    let mut order = Order::submit(new_order());
    apply(
        &mut order,
        exec(report(
            "e0",
            ExecType::PendingNew,
            OrdStatus::PendingNew,
            "0",
            "10",
        )),
    );
    assert_eq!(order.status(), OrdStatus::PendingNew);
}

#[test]
fn a_new_ack_makes_the_order_new() {
    assert_eq!(acked().status(), OrdStatus::New);
}

#[test]
fn a_second_new_ack_is_unexpected() {
    let mut order = acked();
    let err = refuse(
        &mut order,
        exec(report("e1", ExecType::New, OrdStatus::New, "0", "10")),
    );
    assert!(matches!(err, OrderError::UnexpectedExecType { .. }));
}

#[test]
fn a_rejected_order_is_terminal() {
    let mut order = Order::submit(new_order());
    apply(
        &mut order,
        exec(report(
            "e0",
            ExecType::Rejected,
            OrdStatus::Rejected,
            "0",
            "0",
        )),
    );
    assert_eq!(order.status(), OrdStatus::Rejected);
    assert!(order.is_terminal());
    assert_eq!(order.leaves_qty(), Quantity::ZERO);
}

#[test]
fn a_reject_after_the_ack_is_unexpected() {
    let mut order = acked();
    let err = refuse(
        &mut order,
        exec(report(
            "e1",
            ExecType::Rejected,
            OrdStatus::Rejected,
            "0",
            "0",
        )),
    );
    assert!(matches!(err, OrderError::UnexpectedExecType { .. }));
}

#[test]
fn submitting_twice_is_refused() {
    let mut order = acked();
    let err = refuse(&mut order, OrderEvent::Submitted(new_order()));
    assert_eq!(err, OrderError::AlreadySubmitted);
}

// --- Fills -------------------------------------------------------------------

#[test]
fn partial_then_full_fill() {
    let mut order = partially_filled();
    assert_eq!(order.status(), OrdStatus::PartiallyFilled);
    assert_eq!(order.cum_qty(), q("4"));
    assert_eq!(order.leaves_qty(), q("6"));

    apply(
        &mut order,
        exec(trade("e2", OrdStatus::Filled, ("6", "101"), "10", "0")),
    );
    assert_eq!(order.status(), OrdStatus::Filled);
    assert!(order.is_terminal());
    assert_eq!(order.cum_qty(), q("10"));
    assert_eq!(order.leaves_qty(), Quantity::ZERO);
    assert_eq!(order.avg_price(), Some(d("100.6")));
}

#[test]
fn a_fill_before_the_ack_is_accepted() {
    let mut order = Order::submit(new_order());
    apply(
        &mut order,
        exec(trade("e1", OrdStatus::Filled, ("10", "99.5"), "10", "0")),
    );
    assert_eq!(order.status(), OrdStatus::Filled);
}

#[test]
fn a_repeated_exec_id_is_ignored() {
    let mut order = partially_filled();
    let again = trade("e1", OrdStatus::PartiallyFilled, ("4", "100"), "4", "6");
    assert_eq!(apply(&mut order, exec(again)), Applied::Duplicate);
    assert_eq!(order.cum_qty(), q("4"));
}

#[test]
fn an_overfill_is_refused() {
    let mut order = partially_filled();
    let err = refuse(
        &mut order,
        exec(trade("e2", OrdStatus::Filled, ("7", "100"), "11", "0")),
    );
    assert!(matches!(err, OrderError::Overfill { .. }));
}

#[test]
fn a_wrong_cum_qty_is_refused() {
    let mut order = acked();
    let err = refuse(
        &mut order,
        exec(trade(
            "e1",
            OrdStatus::PartiallyFilled,
            ("4", "100"),
            "5",
            "5",
        )),
    );
    assert_eq!(
        err,
        OrderError::QuantityMismatch {
            field: QuantityField::CumQty,
            ours: q("4"),
            reported: q("5"),
        }
    );
}

#[test]
fn a_wrong_leaves_qty_is_refused() {
    let mut order = acked();
    let err = refuse(
        &mut order,
        exec(trade(
            "e1",
            OrdStatus::PartiallyFilled,
            ("4", "100"),
            "4",
            "5",
        )),
    );
    assert!(matches!(
        err,
        OrderError::QuantityMismatch {
            field: QuantityField::LeavesQty,
            ..
        }
    ));
}

#[test]
fn leaves_must_be_zero_on_a_terminal_status() {
    let mut order = acked();
    let err = refuse(
        &mut order,
        exec(report(
            "e1",
            ExecType::Canceled,
            OrdStatus::Canceled,
            "0",
            "10",
        )),
    );
    assert!(matches!(
        err,
        OrderError::QuantityMismatch {
            field: QuantityField::LeavesQty,
            ..
        }
    ));
}

#[test]
fn filled_with_quantity_left_is_refused() {
    let mut order = acked();
    let err = refuse(
        &mut order,
        exec(trade("e1", OrdStatus::Filled, ("4", "100"), "4", "0")),
    );
    assert!(matches!(
        err,
        OrderError::QuantityMismatch {
            field: QuantityField::CumQty,
            ..
        }
    ));
}

#[test]
fn a_trade_needs_last_qty_and_last_px() {
    let mut order = acked();
    let mut no_price = trade("e1", OrdStatus::PartiallyFilled, ("4", "100"), "4", "6");
    no_price.last_px = None;
    assert!(matches!(
        refuse(&mut order, exec(no_price)),
        OrderError::InvalidField {
            field: "LastPx (31)",
            ..
        }
    ));

    let zero_qty = trade("e1", OrdStatus::PartiallyFilled, ("0", "100"), "0", "10");
    assert!(matches!(
        refuse(&mut order, exec(zero_qty)),
        OrderError::InvalidField {
            field: "LastQty (32)",
            ..
        }
    ));
}

#[test]
fn an_ioc_remainder_can_expire_with_the_trade() {
    let mut order = acked();
    apply(
        &mut order,
        exec(trade("e1", OrdStatus::Expired, ("4", "100"), "4", "0")),
    );
    assert_eq!(order.status(), OrdStatus::Expired);
    assert_eq!(order.cum_qty(), q("4"));
    assert_eq!(order.leaves_qty(), Quantity::ZERO);
}

// --- Cancel ------------------------------------------------------------------

#[test]
fn a_cancel_is_pending_then_canceled() {
    let mut order = acked();
    apply(&mut order, cancel("c2"));
    assert_eq!(order.status(), OrdStatus::PendingCancel);
    assert_eq!(order.pending().map(|p| p.kind), Some(RequestKind::Cancel));

    let mut ack = report(
        "e1",
        ExecType::PendingCancel,
        OrdStatus::PendingCancel,
        "0",
        "10",
    );
    ack.cl_ord_id = cl("c2");
    ack.orig_cl_ord_id = Some(cl("c1"));
    apply(&mut order, exec(ack));
    assert_eq!(order.status(), OrdStatus::PendingCancel);

    let mut done = report("e2", ExecType::Canceled, OrdStatus::Canceled, "0", "0");
    done.cl_ord_id = cl("c2");
    apply(&mut order, exec(done));
    assert_eq!(order.status(), OrdStatus::Canceled);
    assert_eq!(order.pending(), None);
    assert_eq!(order.cl_ord_ids(), [cl("c1"), cl("c2")]);
}

#[test]
fn a_rejected_cancel_restores_the_status() {
    let mut order = partially_filled();
    apply(&mut order, cancel("c2"));
    apply(&mut order, rejected("c2"));
    assert_eq!(order.status(), OrdStatus::PartiallyFilled);
    assert_eq!(order.pending(), None);
}

#[test]
fn a_cancel_before_the_ack_survives_the_ack() {
    let mut order = Order::submit(new_order());
    apply(&mut order, cancel("c2"));
    apply(
        &mut order,
        exec(report("e0", ExecType::New, OrdStatus::New, "0", "10")),
    );
    assert_eq!(order.status(), OrdStatus::PendingCancel);
    apply(&mut order, rejected("c2"));
    assert_eq!(order.status(), OrdStatus::New);
}

#[test]
fn a_fill_during_a_cancel_keeps_the_cancel_pending() {
    let mut order = acked();
    apply(&mut order, cancel("c2"));
    apply(
        &mut order,
        exec(trade(
            "e1",
            OrdStatus::PartiallyFilled,
            ("4", "100"),
            "4",
            "6",
        )),
    );
    assert_eq!(order.status(), OrdStatus::PendingCancel);
    assert_eq!(order.cum_qty(), q("4"));

    apply(&mut order, rejected("c2"));
    assert_eq!(order.status(), OrdStatus::PartiallyFilled);
}

#[test]
fn a_full_fill_during_a_cancel_ends_the_cancel() {
    let mut order = acked();
    apply(&mut order, cancel("c2"));
    apply(
        &mut order,
        exec(trade("e1", OrdStatus::Filled, ("10", "100"), "10", "0")),
    );
    assert_eq!(order.status(), OrdStatus::Filled);
    assert_eq!(order.pending(), None);
}

#[test]
fn an_unsolicited_cancel_is_accepted() {
    assert_eq!(canceled().status(), OrdStatus::Canceled);
}

#[test]
fn a_reject_for_an_unknown_request_is_refused() {
    let mut order = acked();
    apply(&mut order, cancel("c2"));
    let err = refuse(&mut order, rejected("c9"));
    assert_eq!(
        err,
        OrderError::NoMatchingRequest {
            cl_ord_id: cl("c9")
        }
    );
}

#[test]
fn one_request_at_a_time() {
    let mut order = acked();
    apply(&mut order, cancel("c2"));
    let err = refuse(&mut order, replace("c3", "8", "99"));
    assert_eq!(err, OrderError::RequestInFlight { pending: cl("c2") });
}

#[test]
fn a_terminal_order_takes_no_requests() {
    let mut order = canceled();
    let err = refuse(&mut order, cancel("c2"));
    assert_eq!(
        err,
        OrderError::Terminal {
            status: OrdStatus::Canceled
        }
    );
}

// --- Replace -----------------------------------------------------------------

#[test]
fn a_replace_is_pending_then_replaced() {
    let mut order = acked();
    apply(&mut order, replace("c2", "8", "99"));
    assert_eq!(order.status(), OrdStatus::PendingReplace);

    let mut ack = report(
        "e1",
        ExecType::PendingReplace,
        OrdStatus::PendingReplace,
        "0",
        "10",
    );
    ack.cl_ord_id = cl("c2");
    apply(&mut order, exec(ack));
    assert_eq!(order.status(), OrdStatus::PendingReplace);

    let mut done = report("e2", ExecType::Replaced, OrdStatus::New, "0", "8");
    done.cl_ord_id = cl("c2");
    done.orig_cl_ord_id = Some(cl("c1"));
    done.order_qty = Some(q("8"));
    done.price = Some(p("99"));
    apply(&mut order, exec(done));
    assert_eq!(order.status(), OrdStatus::New);
    assert_eq!(order.quantity(), q("8"));
    assert_eq!(order.limit_price(), Some(p("99")));
    assert_eq!(order.cl_ord_id(), &cl("c2"));
    assert_eq!(order.pending(), None);
}

#[test]
fn a_replace_takes_the_requested_values_when_the_venue_omits_them() {
    let mut order = acked();
    apply(&mut order, replace("c2", "8", "99"));
    let mut done = report("e1", ExecType::Replaced, OrdStatus::New, "0", "8");
    done.cl_ord_id = cl("c2");
    apply(&mut order, exec(done));
    assert_eq!(order.quantity(), q("8"));
    assert_eq!(order.limit_price(), Some(p("99")));
}

#[test]
fn a_replace_after_a_partial_fill_keeps_the_fills() {
    let mut order = partially_filled();
    apply(&mut order, replace("c2", "6", "100"));
    let mut done = report(
        "e2",
        ExecType::Replaced,
        OrdStatus::PartiallyFilled,
        "4",
        "2",
    );
    done.cl_ord_id = cl("c2");
    done.order_qty = Some(q("6"));
    apply(&mut order, exec(done));
    assert_eq!(order.status(), OrdStatus::PartiallyFilled);
    assert_eq!(order.leaves_qty(), q("2"));
}

#[test]
fn a_replace_below_the_filled_quantity_is_refused() {
    let mut order = partially_filled();
    let err = refuse(&mut order, replace("c2", "3", "100"));
    assert!(matches!(err, OrderError::Overfill { .. }));
}

#[test]
fn a_replace_to_zero_is_refused() {
    let mut order = acked();
    let err = refuse(&mut order, replace("c2", "0", "100"));
    assert!(matches!(err, OrderError::Overfill { .. }));
}

#[test]
fn a_rejected_replace_changes_nothing() {
    let mut order = acked();
    apply(&mut order, replace("c2", "8", "99"));
    apply(&mut order, rejected("c2"));
    assert_eq!(order.status(), OrdStatus::New);
    assert_eq!(order.quantity(), q("10"));
    assert_eq!(order.limit_price(), Some(p("100")));
    assert_eq!(order.cl_ord_id(), &cl("c1"));
}

// --- Other venue events --------------------------------------------------------

#[test]
fn an_order_can_expire() {
    let mut order = acked();
    apply(
        &mut order,
        exec(report(
            "e1",
            ExecType::Expired,
            OrdStatus::Expired,
            "0",
            "0",
        )),
    );
    assert_eq!(order.status(), OrdStatus::Expired);
    assert!(order.is_terminal());
}

#[test]
fn status_only_exec_types_set_their_status() {
    let cases = [
        (ExecType::DoneForDay, OrdStatus::DoneForDay, "0"),
        (ExecType::Calculated, OrdStatus::Calculated, "0"),
        (ExecType::Stopped, OrdStatus::Stopped, "10"),
        (ExecType::Suspended, OrdStatus::Suspended, "10"),
    ];
    for (exec_type, status, leaves) in cases {
        let mut order = acked();
        apply(
            &mut order,
            exec(report("e1", exec_type, status, "0", leaves)),
        );
        assert_eq!(order.status(), status, "{exec_type}");
    }
}

#[test]
fn a_restatement_changes_quantity_and_price() {
    let mut order = acked();
    let mut restated = report("e1", ExecType::Restated, OrdStatus::New, "0", "7");
    restated.order_qty = Some(q("7"));
    restated.price = Some(p("98"));
    apply(&mut order, exec(restated));
    assert_eq!(order.quantity(), q("7"));
    assert_eq!(order.limit_price(), Some(p("98")));
}

#[test]
fn a_restatement_below_the_filled_quantity_is_refused() {
    let mut order = partially_filled();
    let mut restated = report(
        "e2",
        ExecType::Restated,
        OrdStatus::PartiallyFilled,
        "4",
        "0",
    );
    restated.order_qty = Some(q("3"));
    assert!(matches!(
        refuse(&mut order, exec(restated)),
        OrderError::Overfill { .. }
    ));
}

#[test]
fn a_bust_removes_a_fill_and_reopens_the_order() {
    let mut order = partially_filled();
    apply(
        &mut order,
        exec(trade("e2", OrdStatus::Filled, ("6", "101"), "10", "0")),
    );
    let mut bust = report(
        "e3",
        ExecType::TradeCancel,
        OrdStatus::PartiallyFilled,
        "4",
        "6",
    );
    bust.exec_ref_id = Some(ExecId::new("e2"));
    apply(&mut order, exec(bust));
    assert_eq!(order.status(), OrdStatus::PartiallyFilled);
    assert_eq!(order.cum_qty(), q("4"));
    assert_eq!(order.avg_price(), Some(d("100")));
}

#[test]
fn a_correction_replaces_a_fill() {
    let mut order = partially_filled();
    let mut correction = ExecutionReport {
        last_qty: Some(q("3")),
        last_px: Some(p("100.5")),
        ..report(
            "e2",
            ExecType::TradeCorrect,
            OrdStatus::PartiallyFilled,
            "3",
            "7",
        )
    };
    correction.exec_ref_id = Some(ExecId::new("e1"));
    apply(&mut order, exec(correction));
    assert_eq!(order.cum_qty(), q("3"));
    assert_eq!(order.avg_price(), Some(d("100.5")));
}

#[test]
fn a_correction_that_overfills_is_refused() {
    let mut order = partially_filled();
    let mut correction = ExecutionReport {
        last_qty: Some(q("11")),
        last_px: Some(p("100")),
        ..report("e2", ExecType::TradeCorrect, OrdStatus::Filled, "11", "0")
    };
    correction.exec_ref_id = Some(ExecId::new("e1"));
    assert!(matches!(
        refuse(&mut order, exec(correction)),
        OrderError::Overfill { .. }
    ));
}

#[test]
fn a_bust_of_an_unknown_fill_is_refused() {
    let mut order = partially_filled();
    let mut bust = report("e2", ExecType::TradeCancel, OrdStatus::New, "0", "10");
    bust.exec_ref_id = Some(ExecId::new("nope"));
    assert_eq!(
        refuse(&mut order, exec(bust)),
        OrderError::UnknownFill {
            exec_ref_id: ExecId::new("nope")
        }
    );
}

#[test]
fn a_bust_needs_an_exec_ref_id() {
    let mut order = partially_filled();
    let bust = report("e2", ExecType::TradeCancel, OrdStatus::New, "0", "10");
    assert!(matches!(
        refuse(&mut order, exec(bust)),
        OrderError::InvalidField {
            field: "ExecRefID (19)",
            ..
        }
    ));
}

#[test]
fn status_reports_are_never_deduplicated() {
    let mut order = acked();
    for _ in 0..2 {
        let status = report("0", ExecType::OrderStatus, OrdStatus::New, "0", "10");
        assert_eq!(apply(&mut order, exec(status)), Applied::Changed);
    }
}

#[test]
fn a_status_report_that_disagrees_is_refused() {
    let mut order = partially_filled();
    let status = report(
        "0",
        ExecType::OrderStatus,
        OrdStatus::PartiallyFilled,
        "5",
        "5",
    );
    assert!(matches!(
        refuse(&mut order, exec(status)),
        OrderError::QuantityMismatch { .. }
    ));
}

// --- Exhaustive checks --------------------------------------------------------

#[test]
fn every_status_that_does_not_fit_its_exec_type_is_refused() {
    for exec_type in ExecType::ALL {
        for status in OrdStatus::ALL {
            if exec_type.allows_status(status) {
                continue;
            }
            let mut order = acked();
            let err = refuse(&mut order, exec(report("e1", exec_type, status, "0", "10")));
            assert_eq!(
                err,
                OrderError::StatusDoesNotFit {
                    exec_type,
                    ord_status: status
                }
            );
        }
    }
}

#[test]
fn a_terminal_order_refuses_every_exec_type_except_corrections() {
    let corrective = [
        ExecType::TradeCorrect,
        ExecType::TradeCancel,
        ExecType::Restated,
        ExecType::OrderStatus,
    ];
    for exec_type in ExecType::ALL {
        if corrective.contains(&exec_type) {
            continue;
        }
        let status = OrdStatus::ALL
            .into_iter()
            .find(|s| exec_type.allows_status(*s))
            .expect("every exec type allows a status");
        let mut order = canceled();
        let err = refuse(&mut order, exec(report("e1", exec_type, status, "0", "0")));
        assert_eq!(
            err,
            OrderError::Terminal {
                status: OrdStatus::Canceled
            },
            "{exec_type}"
        );
    }
}

#[test]
fn a_terminal_order_accepts_a_matching_status_report() {
    let mut order = canceled();
    let status = report("0", ExecType::OrderStatus, OrdStatus::Canceled, "0", "0");
    assert_eq!(apply(&mut order, exec(status)), Applied::Changed);
}

// --- Replay ---------------------------------------------------------------------

#[test]
fn replay_rebuilds_the_same_order() {
    let events = vec![
        OrderEvent::Submitted(new_order()),
        exec(report("e0", ExecType::New, OrdStatus::New, "0", "10")),
        exec(trade(
            "e1",
            OrdStatus::PartiallyFilled,
            ("4", "100"),
            "4",
            "6",
        )),
        replace("c2", "8", "99"),
        exec(trade(
            "e2",
            OrdStatus::PartiallyFilled,
            ("1", "100"),
            "5",
            "5",
        )),
        rejected("c2"),
        cancel("c3"),
        exec(report(
            "e3",
            ExecType::Canceled,
            OrdStatus::Canceled,
            "5",
            "0",
        )),
    ];

    let mut live = Order::submit(new_order());
    for event in &events[1..] {
        apply(&mut live, event.clone());
    }

    let replayed = Order::replay(&events).expect("replay works");
    assert_eq!(replayed, live);
    assert_eq!(replayed.status(), OrdStatus::Canceled);
    assert_eq!(replayed.cum_qty(), q("5"));
}

#[test]
fn replay_must_start_with_submitted() {
    let events = [cancel("c2")];
    assert_eq!(Order::replay(&events), Err(OrderError::NotSubmitted));
    assert_eq!(Order::replay(&[]), Err(OrderError::NotSubmitted));
}

#[test]
fn replay_stops_at_the_first_bad_event() {
    let events = [OrderEvent::Submitted(new_order()), rejected("c9")];
    assert!(matches!(
        Order::replay(&events),
        Err(OrderError::NoMatchingRequest { .. })
    ));
}

#[test]
fn errors_read_as_sentences() {
    let err = OrderError::Overfill {
        cum_qty: q("11"),
        order_qty: q("10"),
    };
    assert_eq!(
        err.to_string(),
        "filled 11 would exceed the order quantity 10"
    );
}

#[test]
fn accessors_report_the_order() {
    let order = acked();
    assert_eq!(order.id(), OrderId(1));
    assert_eq!(order.instrument(), InstrumentId(7));
    assert_eq!(order.side(), Side::Buy);
    assert!(order.fills().is_empty());
}

#[test]
fn an_unsolicited_replace_is_accepted() {
    let mut order = acked();
    let mut done = report("e1", ExecType::Replaced, OrdStatus::New, "0", "6");
    done.cl_ord_id = cl("venue-7");
    done.order_qty = Some(q("6"));
    apply(&mut order, exec(done));
    assert_eq!(order.quantity(), q("6"));
    assert_eq!(order.limit_price(), Some(p("100")));
    assert_eq!(order.cl_ord_id(), &cl("venue-7"));
    assert_eq!(order.cl_ord_ids(), [cl("c1"), cl("venue-7")]);
}

#[test]
fn a_correction_of_an_unknown_fill_is_refused() {
    let mut order = partially_filled();
    let mut correction = ExecutionReport {
        last_qty: Some(q("3")),
        last_px: Some(p("100")),
        ..report(
            "e2",
            ExecType::TradeCorrect,
            OrdStatus::PartiallyFilled,
            "3",
            "7",
        )
    };
    correction.exec_ref_id = Some(ExecId::new("nope"));
    assert_eq!(
        refuse(&mut order, exec(correction)),
        OrderError::UnknownFill {
            exec_ref_id: ExecId::new("nope")
        }
    );
}

#[test]
fn every_error_reads_as_a_sentence() {
    let cases = [
        (
            OrderError::NotSubmitted,
            "the first event of an order must be Submitted",
        ),
        (
            OrderError::AlreadySubmitted,
            "the order was already submitted",
        ),
        (
            OrderError::Terminal {
                status: OrdStatus::Filled,
            },
            "the order is Filled and cannot change",
        ),
        (
            OrderError::RequestInFlight { pending: cl("c2") },
            "request c2 is still waiting for an answer",
        ),
        (
            OrderError::NoMatchingRequest {
                cl_ord_id: cl("c9"),
            },
            "no request c9 is waiting for an answer",
        ),
        (
            OrderError::StatusDoesNotFit {
                exec_type: ExecType::Trade,
                ord_status: OrdStatus::New,
            },
            "OrdStatus New cannot go with ExecType Trade",
        ),
        (
            OrderError::UnexpectedExecType {
                exec_type: ExecType::New,
                status: OrdStatus::New,
            },
            "ExecType New does not fit an order that is New",
        ),
        (
            OrderError::InvalidField {
                exec_type: ExecType::Trade,
                field: "LastPx (31)",
            },
            "ExecType Trade needs LastPx (31) above zero",
        ),
        (
            OrderError::UnknownFill {
                exec_ref_id: ExecId::new("e9"),
            },
            "no fill with ExecID e9",
        ),
        (
            OrderError::QuantityMismatch {
                field: QuantityField::CumQty,
                ours: q("4"),
                reported: q("5"),
            },
            "CumQty is 4 by our fills but 5 by the venue",
        ),
    ];
    for (err, text) in cases {
        assert_eq!(err.to_string(), text);
    }
}

#[test]
fn a_venue_status_change_during_a_cancel_wins() {
    let mut order = acked();
    apply(&mut order, cancel("c2"));
    apply(
        &mut order,
        exec(report(
            "e1",
            ExecType::DoneForDay,
            OrdStatus::DoneForDay,
            "0",
            "0",
        )),
    );
    assert_eq!(order.status(), OrdStatus::DoneForDay);
    assert!(
        order.pending().is_some(),
        "the cancel still waits for an answer"
    );
}

#[test]
fn a_replace_down_to_the_filled_quantity_is_allowed() {
    let mut order = partially_filled();
    apply(&mut order, replace("c2", "4", "100"));
    assert_eq!(order.status(), OrdStatus::PendingReplace);
}

#[test]
fn fills_are_kept_by_exec_id() {
    let order = partially_filled();
    let fill = order.fills().get(&ExecId::new("e1")).expect("fill e1");
    assert_eq!(fill.quantity, q("4"));
    assert_eq!(fill.price, p("100"));
}
