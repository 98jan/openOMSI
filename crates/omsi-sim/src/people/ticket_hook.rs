//! The plugins' say in a passenger's ticket: who boards the player's bus is reported
//! ([`TicketAsk`], once, when they have decided what they do about a ticket), and a plugin may
//! change it ([`TicketChange`]) before they ask for it at the cash desk - a tariff of zones,
//! a pass that needs no ticket, a voice line of its own. Without a plugin listening nothing
//! changes: the pack's ticket, its price and `Ticket_<n>_<k>`.

use super::*;

/// The longest a passenger at the desk waits for the plugins to have their say (s): the
/// report reaches them in the next frame, so this is only felt when the passenger came
/// in at once.
pub const TICKET_HOLD: f64 = 1.0;

/// The longest text a plugin may give a passenger (a name, a voice line, its tag).
const MAX_TEXT: usize = 256;

/// A passenger boarding the player's bus, as they decided about their ticket.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TicketAsk {
    /// The person (`Person::id`, the id `people.list` gives).
    pub person: u32,
    pub age: f32,
    /// `TICKET_NONE` (nothing to do), `TICKET_STAMP` (a ticket to validate) or `TICKET_BUY`.
    pub kind: u8,
    /// The ticket of the pack they ask for (from 1; 0: none fits their age) and its name
    /// and price.
    pub ticket: u8,
    pub name: String,
    pub price: f32,
    /// The stop they board at: its map object id and its name (the timetable's).
    pub stop: Option<i64>,
    pub stop_name: Option<String>,
    /// Where they ride to: the stop's name, as the timetable names it.
    pub destination: Option<String>,
    /// Where the two are in the duty's trip (from 0), where the trip has them.
    pub stop_index: Option<usize>,
    pub destination_index: Option<usize>,
}

/// What a plugin changes of a passenger's ticket; `None` leaves that part as it is.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TicketChange {
    /// `TICKET_NONE`, `TICKET_STAMP` or `TICKET_BUY`: only before they are in the bus.
    pub kind: Option<u8>,
    /// Another ticket of the pack (from 1; 0: none): its name and price with it.
    pub ticket: Option<u8>,
    /// The name and price asked for, instead of the pack's.
    pub name: Option<String>,
    pub price: Option<f32>,
    /// The voice line said at the desk instead of `Ticket_<n>_<k>` (a file `<name>.wav` of
    /// the passenger's voice folder; where it has none, the usual line). Empty: the usual.
    pub voice: Option<String>,
    /// A text of the plugin's own, kept with the passenger (empty: none).
    pub tag: Option<String>,
}

impl TicketChange {
    fn changes_ticket(&self) -> bool {
        self.kind.is_some() || self.ticket.is_some() || self.name.is_some() || self.price.is_some() || self.voice.is_some()
    }
}

fn text(s: &str) -> Option<String> {
    let s = s.trim();
    (!s.is_empty()).then(|| s.chars().take(MAX_TEXT).collect())
}

impl PeopleSim {
    /// Report passenger `i`'s ticket for the player's bus (they have just decided about it).
    pub fn ask_ticket(&mut self, i: usize) {
        if !self.ticket_hook {
            return;
        }
        let Some(p) = self.pax(i) else { return };
        let stop = p.stop;
        let stop_name = stop.and_then(|s| {
            self.stop_names.as_ref().and_then(|n| n.get(&s).cloned()).or_else(|| self.stops.get(&s).map(|x| if x.alias.trim().is_empty() { x.name.clone() } else { x.alias.clone() }))
        });
        let destination = p.dest.as_ref().map(|d| d.trim().to_string());
        let (mut stop_index, mut destination_index) = (None, None);
        if let Some((trip, _, _)) = &self.duty {
            stop_index = stop.and_then(|s| trip.stops.iter().position(|(id, _, _)| *id == s));
            if let Some(d) = &destination {
                let from = stop_index.map_or(0, |k| k + 1);
                destination_index = trip.stops.iter().enumerate().skip(from).find(|(_, (_, n, _))| n.trim() == d.as_str()).map(|(k, _)| k);
            }
        }
        let ticket = p.ticket_id;
        let name = self.tickets.as_ref().and_then(|t| t.tickets.get((ticket as usize).wrapping_sub(1))).map(|t| t.name.trim().to_string()).unwrap_or_default();
        let ask = TicketAsk {
            person: self.people[i].id,
            age: self.people[i].age,
            kind: p.ticket,
            ticket,
            name,
            price: p.price,
            stop,
            stop_name: stop_name.map(|n| n.trim().to_string()),
            destination,
            stop_index,
            destination_index,
        };
        let now = self.time;
        self.pax_mut(i).unwrap().ticket_hold = Some(now);
        // (a game whose plugins stopped looking must not pile them up)
        if self.ticket_asks.len() < 256 {
            self.ticket_asks.push(ask);
        }
    }

    /// The passengers who decided about their ticket for the player's bus since the last
    /// call. Their requests at the desk wait (a second at most) for `ticket_asks_heard`.
    pub fn take_ticket_asks(&mut self) -> Vec<TicketAsk> {
        std::mem::take(&mut self.ticket_asks)
    }

    /// The plugins have had their frame with these passengers: they ask at the desk now.
    pub fn ticket_asks_heard(&mut self, people: &[u32]) {
        for i in 0..self.people.len() {
            if people.contains(&self.people[i].id) {
                if let Some(p) = self.pax_mut(i) {
                    p.ticket_hold = None;
                }
            }
        }
    }

    /// Change passenger `person`'s ticket (`by`: the plugin). The kind changes only while
    /// they have not got in yet, the ticket only until they have asked for it at the desk;
    /// the tag always. The reason when nothing was changed.
    pub fn set_ticket(&mut self, person: u32, c: &TicketChange, by: &str) -> Result<(), &'static str> {
        let Some(i) = self.people.iter().position(|p| p.id == person) else { return Err("no such person") };
        let pack_len = self.tickets.as_ref().map_or(0, |t| t.tickets.len());
        let (has_sale, has_stamper) = self.player_cabin.as_ref().map_or((false, false), |c| (c.sale.is_some(), !c.stampers.is_empty()));
        let walk = self.boarding.eq_ignore_ascii_case("walk");
        let Some(p) = self.pax(i) else { return Err("not a passenger") };
        if c.changes_ticket() {
            if p.bus != Some(BusId::Player) {
                return Err("not a passenger of the player's bus");
            }
            // (they decide on their way to the door: `ask_ticket`)
            let outside = p.inside.is_none() && p.task == Task::WalkingToBus;
            let at_desk_yet = p.inside.is_some() && p.task == Task::InBusToPlace && p.ticket == TICKET_BUY && p.sub < 4;
            if !outside && !at_desk_yet {
                return Err(if p.inside.is_some() { "already asked for the ticket" } else { "not boarding the bus yet" });
            }
            if let Some(kind) = c.kind.filter(|k| *k != p.ticket) {
                if !outside {
                    return Err("already in the bus");
                }
                match kind {
                    TICKET_BUY if !has_sale || walk => return Err("the bus sells no tickets"),
                    TICKET_STAMP if !has_stamper => return Err("the bus has no validator"),
                    TICKET_NONE | TICKET_STAMP | TICKET_BUY => {}
                    _ => return Err("no such kind"),
                }
            }
            if c.ticket.is_some_and(|t| t as usize > pack_len) {
                return Err("no such ticket");
            }
        }
        let pack = self.tickets.clone();
        let p = self.pax_mut(i).unwrap();
        if c.changes_ticket() {
            if let Some(k) = c.kind {
                p.ticket = k;
            }
            if let Some(t) = c.ticket {
                p.ticket_id = t;
                p.price = pack.as_ref().and_then(|pk| pk.tickets.get((t as usize).wrapping_sub(1))).map_or(0.0, |x| x.value);
                p.ticket_name = None;
            }
            if let Some(n) = &c.name {
                p.ticket_name = text(n);
            }
            if let Some(v) = c.price {
                p.price = if v.is_finite() { v.max(0.0) } else { 0.0 };
            }
            if let Some(v) = &c.voice {
                p.ticket_voice = text(v);
            }
            p.ticket_by = text(by);
            p.ticket_hold = None;
        }
        if let Some(t) = &c.tag {
            p.ticket_tag = text(t);
        }
        Ok(())
    }

    /// Say the ticket asked for: the plugin's line where the passenger's voice has it, else
    /// `Ticket_<id>_<k>`.
    pub fn say_ticket(&mut self, i: usize, id: usize, k: u64, custom: Option<&str>) {
        if let Some(line) = custom.filter(|l| self.voice_file(i, l).is_some()) {
            let line = line.to_string();
            self.say_line(i, &line, false, true);
            return;
        }
        self.say_line(i, &format!("Ticket_{id}_{k}"), false, true);
    }
}
