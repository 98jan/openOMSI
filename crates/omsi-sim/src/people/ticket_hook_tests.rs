//! The plugins' say in the passengers' tickets (`ticket_hook`).

use super::*;

/// A passenger of the player's bus on the way to its door, about to buy ticket 1.
fn boarding(ty: &Arc<HumanType>, id: u32, task: Task) -> Person {
    let mut pax = Pax::new(1.1, 0.5);
    pax.bus = Some(BusId::Player);
    pax.stop = Some(77);
    pax.dest = Some("Nový Bor".into());
    pax.task = task;
    pax.ticket = TICKET_BUY;
    pax.ticket_id = 1;
    pax.price = 2.7;
    Person {
        id,
        ty: ty.clone(),
        variant: 0,
        position: DVec3::ZERO,
        heading: 0.0,
        lheading: 0.0,
        place: Place::Ground,
        vel: DVec2::ZERO,
        pace: 1.1,
        activity: Activity::Stand,
        anim: OmsiAnim::default(),
        state: State::Pax(Box::new(pax)),
        t_state: 0.0,
        interior: 0.0,
        tilt: Mat4::IDENTITY,
        age: 40.0,
        stuck: 0.0,
        ghost: 0.0,
        car_wait: 0.0,
        detour: 0.0,
        detour_side: 0.0,
        why: "",
        puppet: None,
        remote: false,
    }
}

fn pack() -> omsi_content::tickets::TicketPack {
    let t = |name: &str, value: f32| omsi_content::tickets::Ticket { name: name.into(), age_min: 14, age_max: 200, value, probability: 1.0, ..Default::default() };
    omsi_content::tickets::TicketPack { tickets: vec![t("Fahrschein", 2.7), t("Kurzstrecke", 1.4)], ..Default::default() }
}

#[test]
fn plugins_hear_of_a_ticket_and_change_it_until_it_is_asked_for() {
    let dir = std::env::temp_dir().join(format!("omsi-ticket-hook-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("person.hum"), "[model]\nmodel.cfg\n").unwrap();
    std::fs::write(dir.join("model.cfg"), "").unwrap();
    let ty = Arc::new(HumanType::load(&dir.join("person.hum")).unwrap());
    std::fs::remove_dir_all(&dir).unwrap();
    let mut h = PeopleSim::new(Path::new("/nonexistent"), 200);
    h.tickets = Some(Arc::new(pack()));
    h.people = vec![boarding(&ty, 1, Task::WalkingToBus), boarding(&ty, 2, Task::WaitingForBus), boarding(&ty, 3, Task::WalkingToBus)];
    h.stop_names = Some([(77, "Střížkov".to_string())].into_iter().collect());
    h.duty = Some((Arc::new(DutyTrip { name: "1".into(), departure: 0.0, terminus: "Nový Bor".into(), public: true, stops: vec![(70, "Ládví".into(), true), (77, "Střížkov".into(), true), (80, "Mladá Boleslav".into(), true), (90, "Nový Bor".into(), true)] }), 1, false));

    // nobody listens: nothing is reported, nobody waits
    h.ask_ticket(0);
    assert!(h.take_ticket_asks().is_empty());
    assert_eq!(h.pax(0).unwrap().ticket_hold, None);

    // a plugin listens: the passenger is reported with their ride
    h.ticket_hook = true;
    h.ask_ticket(0);
    let asks = h.take_ticket_asks();
    assert_eq!(asks.len(), 1);
    let a = &asks[0];
    assert_eq!((a.person, a.kind, a.ticket, a.name.as_str(), a.price), (1, TICKET_BUY, 1, "Fahrschein", 2.7));
    assert_eq!((a.stop, a.stop_name.as_deref(), a.destination.as_deref()), (Some(77), Some("Střížkov"), Some("Nový Bor")));
    assert_eq!((a.stop_index, a.destination_index), (Some(1), Some(3)));
    assert!(h.pax(0).unwrap().ticket_hold.is_some(), "the request at the desk waits for the plugins");
    h.ticket_asks_heard(&[1]);
    assert_eq!(h.pax(0).unwrap().ticket_hold, None);

    // the plugin prices the ride by its zones
    let zones = TicketChange { name: Some("IDOL 4001".into()), price: Some(64.0), voice: Some("Zone_4001".into()), tag: Some("telmax".into()), ..Default::default() };
    assert_eq!(h.set_ticket(1, &zones, "telmax"), Ok(()));
    let p = h.pax(0).unwrap();
    assert_eq!((p.ticket_name.as_deref(), p.price, p.ticket_voice.as_deref()), (Some("IDOL 4001"), 64.0, Some("Zone_4001")));
    assert_eq!((p.ticket_by.as_deref(), p.ticket_tag.as_deref(), p.ticket_id), (Some("telmax"), Some("telmax"), 1));
    // another ticket of the pack brings its own name and price
    assert_eq!(h.set_ticket(1, &TicketChange { ticket: Some(2), ..Default::default() }, "telmax"), Ok(()));
    let p = h.pax(0).unwrap();
    assert_eq!((p.ticket_id, p.ticket_name.as_deref(), p.price), (2, None, 1.4));
    assert_eq!(h.set_ticket(1, &TicketChange { ticket: Some(9), ..Default::default() }, "x"), Err("no such ticket"));
    assert_eq!(h.set_ticket(42, &zones, "x"), Err("no such person"));

    // somebody still waiting has decided nothing yet; a tag is kept all the same
    assert_eq!(h.set_ticket(2, &zones, "x"), Err("not boarding the bus yet"));
    assert_eq!(h.set_ticket(2, &TicketChange { tag: Some("seen".into()), ..Default::default() }, "x"), Ok(()));
    assert_eq!(h.pax(1).unwrap().ticket_tag.as_deref(), Some("seen"));

    // a pass instead of a ticket, before they are in; buying needs a cash desk
    let pass = TicketChange { kind: Some(TICKET_NONE), ..Default::default() };
    assert_eq!(h.set_ticket(3, &pass, "x"), Ok(()));
    assert_eq!(h.pax(2).unwrap().ticket, TICKET_NONE);
    let buy = TicketChange { kind: Some(TICKET_BUY), ..Default::default() };
    assert_eq!(h.set_ticket(3, &buy, "x"), Err("the bus sells no tickets"));

    // in the bus on the way to the desk: the ticket still, not the kind; once asked, nothing
    {
        let p = h.pax_mut(0).unwrap();
        p.inside = Some(BusId::Player);
        p.task = Task::InBusToPlace;
        p.sub = 3;
    }
    assert_eq!(h.set_ticket(1, &pass, "x"), Err("already in the bus"));
    assert_eq!(h.set_ticket(1, &zones, "x"), Ok(()));
    h.pax_mut(0).unwrap().sub = 4;
    assert_eq!(h.set_ticket(1, &zones, "x"), Err("already asked for the ticket"));
}
