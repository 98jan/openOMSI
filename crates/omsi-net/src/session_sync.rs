//! Script textures, sound events and variable synchronization for a session.
use super::*;

impl LanSession {
    /// Released surfaces of our own vehicle. The transport retains an acknowledged
    /// baseline per subscriber and sends only pixel changes after initialization.
    pub fn set_script_textures(&mut self, table: u32, images: Vec<Option<scripttex::Image>>) {
        self.script_textures.set_local(table, images);
    }

    pub fn clear_script_textures(&mut self) {
        self.script_textures.clear_local();
    }

    pub fn subscribe_script_textures(&mut self, owner: u32, table: u32) -> u64 {
        self.script_textures.subscribe(owner, table)
    }

    pub fn unsubscribe_script_textures(&mut self, owner: u32) {
        self.script_textures.unsubscribe(owner);
    }

    pub fn take_script_textures(&mut self) -> Vec<scripttex::Incoming> {
        self.script_textures.take_incoming()
    }

    pub fn publish_sound(&mut self, event: sounds::Event) {
        if !self.connected || (event.ai != 0 && self.role != Role::Host) { return; }
        let peers: Vec<u32> = self.peers.keys().copied().collect();
        self.sounds.publish(&peers, event);
    }

    pub fn take_sounds(&mut self) -> Vec<sounds::Incoming> {
        self.sounds.take_incoming().into_iter().filter(|s| s.event.ai == 0 || s.owner == 1).collect()
    }

    pub(super) fn route_sound(&self, data: &[u8]) {
        let Some(a) = sounds::address(data, PROTOCOL as u8) else { return; };
        match self.role {
            Role::Host => if let Some(to) = self.peers.get(&a.to).and_then(|p| p.addr) { self.send(data, to); },
            Role::Client => if let Some(host) = self.host { self.send(data, host); },
        }
    }

    pub(super) fn on_sound(&mut self, data: &[u8], from: SocketAddr) {
        let Some(a) = sounds::address(data, PROTOCOL as u8) else { return; };
        if a.from == self.my_id || a.from == 0 || a.to == 0 { return; }
        match self.role {
            Role::Host => {
                if self.checked_peer(a.from, from, SOUND_RATE, PeerTraffic::Sounds).is_none() { return; }
                if a.to != self.my_id { self.route_sound(data); return; }
            }
            Role::Client => if a.to != self.my_id || !self.peers.contains_key(&a.from) { return; },
        }
        if let Some(reply) = self.sounds.receive(PROTOCOL as u8, self.my_id, data) { self.route_sound(&reply); }
    }

    pub(super) fn route_script_texture(&self, data: &[u8]) {
        let Some(a) = scripttex::address(data, PROTOCOL as u8) else { return };
        match self.role {
            Role::Host => {
                if let Some(to) = self.peers.get(&a.to).and_then(|p| p.addr) {
                    self.send(data, to);
                }
            }
            Role::Client => {
                if let Some(host) = self.host { self.send(data, host); }
            }
        }
    }

    pub(super) fn on_script_texture(&mut self, data: &[u8], from: SocketAddr) {
        let Some(a) = scripttex::address(data, PROTOCOL as u8) else { return };
        if a.from == self.my_id || a.from == 0 || a.to == 0 { return; }
        match self.role {
            Role::Host => {
                if self.checked_peer(a.from, from, TEXTURE_RATE, PeerTraffic::Textures).is_none() { return; }
                if a.to != self.my_id {
                    self.route_script_texture(data);
                    return;
                }
            }
            Role::Client => {
                if a.to != self.my_id || !self.peers.contains_key(&a.from) { return; }
            }
        }
        for reply in self.script_textures.receive(PROTOCOL as u8, self.my_id, data) {
            self.route_script_texture(&reply);
        }
    }

    /// Every script variable of our vehicle (see `vars.rs`): `float_ids[k]` holds
    /// `floats[k]`, the same for the strings, `table` names them (a hash of their names). Called
    /// every frame; it sends what is due.
    #[allow(clippy::too_many_arguments)]
    pub fn send_vars(&mut self, table: u32, float_ids: &[u16], floats: &[f32], string_ids: &[u16], strings: &[String], dt: f32) {
        let msgs = self.var_sender.tick(PROTOCOL as u8, self.my_id, table, float_ids, floats, string_ids, strings, dt);
        for m in msgs {
            match self.role {
                Role::Host => self.broadcast(&m, None),
                Role::Client => {
                    if let Some(h) = self.host {
                        self.send(&m, h);
                    }
                }
            }
        }
    }

    /// Model animation variables, sampled on the same clock as the vehicle pose.
    pub fn send_animation_vars(&mut self, table: u32, values: &[(u16, f32)], dt: f32) {
        let sent_ms = self.stamp_ms();
        for packet in self.animation_sender.tick(PROTOCOL as u8, self.my_id, table, sent_ms, values, dt) {
            match self.role {
                Role::Host => self.broadcast(&packet, None),
                Role::Client => { if let Some(host) = self.host { self.send(&packet, host); } }
            }
        }
    }

    /// Repeat the name manifest using independently recovered string fragments.
    pub fn send_var_schema(&mut self, table: u32, schema: &str, dt: f32) {
        let texts = [schema.to_owned()];
        for mut packet in self.schema_sender.tick(PROTOCOL as u8, self.my_id, table, &[], &[], &[0], &texts, dt) {
            packet[0] = var_schema::MAGIC;
            match self.role {
                Role::Host => self.broadcast(&packet, None),
                Role::Client => { if let Some(host) = self.host { self.send(&packet, host); } }
            }
        }
    }

    pub fn take_var_schemas(&mut self) -> Vec<vars::VarsIn> { std::mem::take(&mut self.schemas_in) }

    /// Host-only AI script context pages, alongside the shared world poses.
    pub fn send_ai_context(&self, to: u32, data: &[u8]) {
        if self.role != Role::Host || data.len() > MAX_DATAGRAM { return; }
        if let Some(addr) = self.peers.get(&to).and_then(|p| p.addr) { self.send(data, addr); }
    }

    pub fn take_ai_contexts(&mut self) -> Vec<Vec<u8>> { std::mem::take(&mut self.ai_context_in) }

    /// The others' vehicle variables that came since the last call, oldest first.
    pub fn take_vars(&mut self) -> Vec<vars::VarsIn> {
        std::mem::take(&mut self.vars_in)
    }

    pub(super) fn on_vars(&mut self, data: &[u8], from: SocketAddr) {
        self.on_variable_packet(data, from, false);
    }

    pub(super) fn on_variable_packet(&mut self, data: &[u8], from: SocketAddr, schema: bool) {
        let Some(v) = vars::decode(data, PROTOCOL as u8) else { return };
        if v.id == self.my_id || v.id == 0 {
            return;
        }
        match self.role {
            // (as a state: only a player we welcomed, from its address, within its rate)
            Role::Host => {
                let (rate, traffic) = if !schema && v.sent_ms != 0 {
                    (ANIMATION_RATE, PeerTraffic::Animations)
                } else {
                    (VAR_RATE, PeerTraffic::Variables)
                };
                if self.checked_peer(v.id, from, rate, traffic).is_none() {
                    return;
                }
                if schema {
                    let mut packet = data.to_vec();
                    packet[0] = var_schema::MAGIC;
                    self.broadcast(&packet, Some(v.id));
                } else {
                    self.broadcast(data, Some(v.id));
                }
            }
            Role::Client => {
                if !self.peers.contains_key(&v.id) {
                    return;
                }
            }
        }
        let receiver = if schema { &mut self.schema_receiver } else { &mut self.var_receiver };
        let Some(v) = receiver.receive(data, PROTOCOL as u8) else { return };
        if schema {
            if self.schemas_in.len() < 128 && !v.strings.is_empty() { self.schemas_in.push(v); }
            return;
        }
        // (a game that stopped reading does not pile them up without end)
        if self.vars_in.len() < 1024 {
            self.vars_in.push(v);
        }
    }

}
