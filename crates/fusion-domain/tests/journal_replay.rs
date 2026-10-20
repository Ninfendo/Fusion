//! Orders rebuilt from the journal equal the live orders: the basis for
//! restart recovery and for backtests that run the same code as live.

use fusion_domain::{
    ClOrdId, Clock, ExecId, ExecType, ExecutionReport, InstrumentId, Journal, ManualClock,
    MemoryJournal, NewOrder, OrdStatus, Order, OrderEvent, OrderId, Quantity, Side, Timestamp,
};

fn q(s: &str) -> Quantity {
    s.parse().expect("valid quantity")
}

fn report(
    exec_id: &str,
    exec_type: ExecType,
    status: OrdStatus,
    cum: &str,
    leaves: &str,
) -> OrderEvent {
    OrderEvent::Execution(Box::new(ExecutionReport {
        exec_id: ExecId::new(exec_id),
        exec_type,
        ord_status: status,
        cl_ord_id: ClOrdId::new("c1"),
        orig_cl_ord_id: None,
        exec_ref_id: None,
        order_qty: None,
        price: None,
        cum_qty: q(cum),
        leaves_qty: q(leaves),
        last_qty: (exec_type == ExecType::Trade).then(|| q("2")),
        last_px: (exec_type == ExecType::Trade).then(|| "100".parse().expect("valid price")),
        transact_time: Timestamp::from_unix_nanos(0),
    }))
}

#[test]
fn an_order_rebuilt_from_the_journal_equals_the_live_order() {
    let clock = ManualClock::new(Timestamp::from_unix_nanos(1_000));
    let mut journal = MemoryJournal::new();

    let events = [
        OrderEvent::Submitted(NewOrder {
            id: OrderId(42),
            cl_ord_id: ClOrdId::new("c1"),
            instrument: InstrumentId(1),
            side: Side::Sell,
            quantity: q("4"),
            limit_price: Some("100".parse().expect("valid price")),
        }),
        report("e0", ExecType::New, OrdStatus::New, "0", "4"),
        report("e1", ExecType::Trade, OrdStatus::PartiallyFilled, "2", "2"),
        report("e2", ExecType::Trade, OrdStatus::Filled, "4", "0"),
    ];

    // Live: journal each event, then apply it.
    let mut live: Option<Order> = None;
    for (step, event) in events.iter().enumerate() {
        clock
            .advance_to(Timestamp::from_unix_nanos(1_000 + step as i64))
            .expect("time moves forward");
        journal.append(clock.now(), event.clone()).expect("append");
        match (&mut live, event) {
            (None, OrderEvent::Submitted(new)) => live = Some(Order::submit(new.clone())),
            (Some(order), event) => {
                order.apply(event).expect("event applies");
            }
            (None, _) => panic!("the first event must be Submitted"),
        }
    }

    // Restart: rebuild from the journal alone.
    let mut recorded = Vec::new();
    journal
        .replay(1, &mut |entry| recorded.push(entry.event.clone()))
        .expect("replay");
    let rebuilt = Order::replay(&recorded).expect("rebuild");

    let live = live.expect("order exists");
    assert_eq!(rebuilt, live);
    assert_eq!(rebuilt.status(), OrdStatus::Filled);
    assert_eq!(journal.last_seq(), 4);
}
