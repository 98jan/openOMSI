//! What the simulation hands the view: plain data, owned and `Send`, never a renderer call.
//!
//! The simulation's calls that put a car on the road, take one off, couple or uncouple its
//! parts or let a parked car drive off write down a [`TrafficEvent`] instead of making or
//! letting go the car's renders themselves; the people's [`BodyOp`](crate::people::BodyOp)s
//! are the same for somebody appearing or going. omsi-app's view sync replays them in the
//! order they happened (`view_sync::apply_traffic`, `view_sync::apply_people`) - right
//! after the call that made them, where the renderer calls used to be made, so that the
//! renderer sees the same calls in the same order. Being data, they can also be handed
//! from a simulation thread to the drawing thread.

use crate::VehicleType;
use std::sync::Arc;

/// Something the traffic did that the renderer has to follow, in the order it happened.
/// Car ids are the traffic's (`AiCar::id`), stable for as long as the car is on the road.
pub enum TrafficEvent {
    /// Car `id` was put on the road: the render of its body (`ty` in paint `scheme`), then
    /// those of its coupled parts in their order (type and paint each).
    Spawned { id: u64, ty: Arc<VehicleType>, scheme: Option<usize>, trailers: Vec<(Arc<VehicleType>, Option<usize>)> },
    /// Car `id` was taken off the road: its renders go back to the world now.
    Released { id: u64 },
    /// Car `id` went in the traffic's step (or parked): its renders go back to the world at
    /// the next view sync.
    Retired { id: u64 },
    /// Car `id` is made anew under its id (a train turned round): its renders are put
    /// aside, to be let go by [`TrafficEvent::DetachedReleased`].
    Detached { id: u64 },
    /// The renders put aside for car `id` go back to the world now.
    DetachedReleased { id: u64 },
    /// Another part of type `ty` was coupled behind car `id` (its own paint).
    TrailerAdded { id: u64, ty: Arc<VehicleType> },
    /// Car `id`'s coupled parts were let go (it gets others).
    TrailersReleased { id: u64 },
    /// A parked car drove off: the renderer instances of its object are hidden.
    ParkedDeparted { instances: Vec<usize> },
    /// A car finished parking: the renderer instances of the parked object that stands in
    /// its place are shown again.
    ParkedReturned { instances: Vec<usize> },
}

/// A run of entries in one of [`FrameSnapshot`]'s flat buffers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Span {
    pub start: u32,
    pub len: u32,
}

impl Span {
    fn range(self) -> std::ops::Range<usize> {
        self.start as usize..(self.start + self.len) as usize
    }

    /// The span of what `buf` gets from `items`, appended to it.
    fn push<T: Clone>(buf: &mut Vec<T>, items: &[T]) -> Span {
        let start = buf.len() as u32;
        buf.extend_from_slice(items);
        Span { start, len: items.len() as u32 }
    }
}

/// What the renderer shows of the traffic at the end of a step: where every AI car's parts
/// stand and how each of their meshes is drawn - owned data, `Send`, no borrow into the
/// simulation, so that it can be handed to the drawing thread while the simulation makes
/// the next one. Its buffers are kept from one snapshot to the next ([`FrameSnapshot::clear`]):
/// a car's meshes and their per-slot values lie in flat buffers, a run of each per car.
#[derive(Default)]
pub struct FrameSnapshot {
    /// The traffic's clock when it was taken (s).
    pub sim_time: f64,
    /// The cars in the traffic's order.
    pub cars: Vec<CarFrame>,
    parts: Vec<PartFrame>,
    meshes: Vec<MeshFrame>,
    alpha: Vec<f32>,
    light: Vec<f32>,
    night: Vec<f32>,
    uv: Vec<[f32; 2]>,
}

/// One AI car of a [`FrameSnapshot`].
#[derive(Clone, Copy, Debug)]
pub struct CarFrame {
    /// The traffic's id of the car (`AiCar::id`).
    pub id: u64,
    /// Whether it is drawn at all (`VehicleInstance::ai_visuals`); a car out of sight has no
    /// parts in the snapshot.
    pub shown: bool,
    parts: Span,
}

/// A vehicle body or a coupled part of a car: where it stands and its meshes.
#[derive(Clone, Copy, Debug)]
pub struct PartFrame {
    pub position: glam::DVec3,
    meshes: Span,
}

/// How one mesh of a vehicle is drawn: its transform relative to the vehicle's position,
/// whether it shows (its `[visible]` variable and the views it is drawn in), and its
/// per-material-slot values (see `MeshProps`).
#[derive(Clone, Copy, Debug)]
pub struct MeshFrame {
    pub local: glam::Mat4,
    pub visible: bool,
    pub interior: f32,
    alpha: Span,
    light: Span,
    night: Span,
    uv: Span,
}

impl FrameSnapshot {
    /// Empty it for the next one, keeping its buffers.
    pub fn clear(&mut self) {
        self.sim_time = 0.0;
        self.cars.clear();
        self.parts.clear();
        self.meshes.clear();
        self.alpha.clear();
        self.light.clear();
        self.night.clear();
        self.uv.clear();
    }

    /// Add car `id` (`vehicle` with its coupled parts), as the view sync draws an AI car:
    /// the meshes of a car out of sight are left out.
    pub fn add_car(&mut self, id: u64, vehicle: &crate::VehicleInstance) {
        let start = self.parts.len() as u32;
        if vehicle.ai_visuals {
            let meshes = (0..vehicle.mesh_props.len()).map(|i| (vehicle.mesh_local_transform(i), &vehicle.mesh_props[i]));
            self.add_part(vehicle.position, &vehicle.ty, meshes);
            for t in &vehicle.trailers {
                let meshes = (0..t.mesh_props.len()).map(|i| (t.mesh_local_transform(i), &t.mesh_props[i]));
                self.add_part(t.position, &t.ty, meshes);
            }
        }
        let parts = Span { start, len: self.parts.len() as u32 - start };
        self.cars.push(CarFrame { id, shown: vehicle.ai_visuals, parts });
    }

    fn add_part<'a>(&mut self, position: glam::DVec3, ty: &VehicleType, meshes: impl Iterator<Item = (glam::Mat4, &'a crate::vehicle::MeshProps)>) {
        let start = self.meshes.len() as u32;
        for (i, (local, p)) in meshes.enumerate() {
            // (a mesh drawn only from some views: an AI car is seen from outside)
            let vp = ty.meshes.get(i).map(|m| ty.model.meshes[m.def_index].viewpoint).unwrap_or(0);
            let frame = MeshFrame {
                local,
                visible: p.visible && (vp == 0 || vp & 4 != 0),
                interior: p.interior,
                alpha: Span::push(&mut self.alpha, &p.slot_alpha),
                light: Span::push(&mut self.light, &p.slot_light),
                night: Span::push(&mut self.night, &p.slot_night),
                uv: Span::push(&mut self.uv, &p.slot_uv),
            };
            self.meshes.push(frame);
        }
        let meshes = Span { start, len: self.meshes.len() as u32 - start };
        self.parts.push(PartFrame { position, meshes });
    }

    /// The parts of `car`: its body first, then its coupled parts in their order.
    pub fn parts(&self, car: &CarFrame) -> &[PartFrame] {
        &self.parts[car.parts.range()]
    }

    /// The meshes of `part`, in the vehicle type's order.
    pub fn meshes(&self, part: &PartFrame) -> &[MeshFrame] {
        &self.meshes[part.meshes.range()]
    }

    /// The alpha multiplier of each material slot of `m`.
    pub fn slot_alpha(&self, m: &MeshFrame) -> &[f32] {
        &self.alpha[m.alpha.range()]
    }

    /// The `[matl_lightmap]` strength of each material slot of `m`.
    pub fn slot_light(&self, m: &MeshFrame) -> &[f32] {
        &self.light[m.light.range()]
    }

    /// Whether the `[matl_item]` of each material slot of `m` shows (0/1).
    pub fn slot_night(&self, m: &MeshFrame) -> &[f32] {
        &self.night[m.night.range()]
    }

    /// The `[texcoordtransX/Y]` offset of each material slot of `m`.
    pub fn slot_uv(&self, m: &MeshFrame) -> &[[f32; 2]] {
        &self.uv[m.uv.range()]
    }
}

impl crate::ai_traffic::TrafficSim {
    /// The cars as they are drawn now (see [`FrameSnapshot`]), into `out` (cleared first).
    pub fn snapshot_into(&self, out: &mut FrameSnapshot) {
        out.clear();
        out.sim_time = self.time as f64;
        for c in &self.cars {
            out.add_car(c.id, &c.vehicle);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn handed_across_threads<T: Send + 'static>() {}

    /// The events are handed from the simulation to the view, possibly across threads.
    #[test]
    fn events_are_owned_and_send() {
        handed_across_threads::<TrafficEvent>();
        handed_across_threads::<Vec<TrafficEvent>>();
        handed_across_threads::<crate::people::BodyOp>();
        handed_across_threads::<FrameSnapshot>();
    }

    /// The flat buffers hand every mesh its own run of slot values, and a cleared snapshot
    /// keeps nothing of the last one.
    #[test]
    fn spans_cut_the_flat_buffers() {
        let mut buf = vec![9.0f32];
        let a = Span::push(&mut buf, &[1.0, 2.0]);
        let b = Span::push(&mut buf, &[]);
        let c = Span::push(&mut buf, &[3.0]);
        assert_eq!(&buf[a.range()], &[1.0, 2.0]);
        assert!(buf[b.range()].is_empty());
        assert_eq!(&buf[c.range()], &[3.0]);
        let mut snap = FrameSnapshot::default();
        snap.cars.push(CarFrame { id: 7, shown: false, parts: Span::default() });
        snap.sim_time = 3.0;
        snap.clear();
        assert!(snap.cars.is_empty());
        assert_eq!(snap.sim_time, 0.0);
    }
}
