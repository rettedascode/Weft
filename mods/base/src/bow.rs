//! The bow: held down with the use key it draws, let go it shoots an arrow from the inventory,
//! harder the longer it was drawn (a second for full power). Arrows fly on their own path with
//! gravity and drag, hurt what they hit and stick in blocks, where their shooter can pick them
//! up again. Flint for their heads comes from gravel.

use std::sync::Mutex;
use std::time::Instant;

use weft_sdk::*;

pub static BOW: Item = Item::named("bow");
pub static ARROW_ITEM: Item = Item::named("arrow");
pub static FLINT: Item = Item::named("flint");
pub static ARROW: Entity = Entity::named("arrow");

value! {
    /// A flying arrow: who shot it, how hard it hits, whether it sticks in a block (and since
    /// how many ticks it exists), and whether it can be picked up (not those shot in creative).
    struct Flight { shooter: u64, damage: f32, stuck: u32, age: u32, pickup: u32 }
}
static FLIGHT: Component<Flight> = Component::named("flight");

/// Client to server: the bow was let go after being drawn this many seconds.
static SHOOT: Channel<f32> = Channel::named("bow_shoot");

/// Blocks per second at full draw, and the pull of gravity and the drag on the way (vanilla's
/// 3 blocks and 0.05 a tick, 1% drag a tick).
const SPEED: f64 = 60.0;
const GRAVITY: f64 = 20.0;
const DRAG: f64 = 0.99;
const TICK: f64 = 1.0 / 20.0;
/// Hearts of damage at full draw, in half hearts.
const DAMAGE: f32 = 9.0;
/// Ticks an arrow lies around before it goes, stuck or not.
const LIFETIME: u32 = 1200;

/// How drawn the bow is after `secs`, 0..1 (vanilla's curve).
fn power(secs: f32) -> f32 {
    ((secs * secs + 2.0 * secs) / 3.0).min(1.0)
}

pub fn register(r: &mut Registry) {
    r.item(&BOW).max_stack(1).durability(384).tab(&crate::tabs::COMBAT);
    r.item(&ARROW_ITEM).tag("arrows").tab(&crate::tabs::COMBAT);
    r.item(&FLINT).tag("flint").tab(&crate::tabs::INGREDIENTS);
    r.shaped(&crate::recipes::CRAFTING, "bow", 1, &[" ts", "t s", " ts"], &[('t', "#stick"), ('s', "#string")]);
    // String for the fletching: there are no feathers.
    r.shaped(&crate::recipes::CRAFTING, "arrow", 4, &["f", "t", "s"], &[('f', "flint"), ('t', "#stick"), ('s', "#string")]);
    r.loot_modify(&crate::blocks::GRAVEL).add_pool(Pool::new().when(Cond::Harvested).when(Cond::Chance(0.1)).item("flint"));

    // A shaft with a stone head toward -z (forward) and two crossed vanes at the back, turned
    // by the engine with the arrow's yaw and, as its head, its pitch.
    let head = Part::new("head", [0.0, 2.0, 0.0])
        .cube([-0.5, -0.5, -8.0], [1.0, 1.0, 16.0], [0.0, 0.0])
        .cube([-1.0, -1.0, -9.5], [2.0, 2.0, 2.0], [32.0, 0.0])
        .cube([-0.1, -1.0, 3.5], [0.2, 2.0, 4.5], [32.0, 8.0])
        .cube([-1.0, -0.1, 3.5], [2.0, 0.2, 4.5], [32.0, 8.0]);
    r.entity(&ARROW).size(0.25, 0.25).health(1.0).invulnerable().unpushable().kinematic().texture("arrow").model(Model::from(vec![head]).uv(64, 32)).on_tick(fly);
    r.entity_component(&FLIGHT, Flight { shooter: 0, damage: 0.0, stuck: 0, age: 0, pickup: 0 }, component_flags::SAVE);
    r.attach(&ARROW, &FLIGHT);

    r.packet(&SHOOT, |p, secs| shoot(&p.world(), p.player(), secs));
    client(r);
}

/// Shoots one arrow from `player`'s bow, drawn for `secs`.
fn shoot(w: &World, player: u64, secs: f32) {
    let f = power(secs);
    if f < 0.1 {
        return;
    }
    let hotbar = w.inventory(player).group("hotbar");
    let slot = w.selected_slot(player);
    let bow = hotbar.get(slot);
    if bow.item != BOW.id() {
        return;
    }
    let creative = w.creative(player);
    if !creative {
        let inv = w.inventory(player);
        let Some(i) = (0..inv.size()).find(|&i| inv.get(i).item == ARROW_ITEM.id()) else { return };
        inv.set(i, inv.get(i).shrink(1));
        let worn = bow.damage() + 1;
        hotbar.set(slot, if worn >= DURABILITY.on_item(bow.item).unwrap_or(u32::MAX) { ItemStack::EMPTY } else { bow.with_damage(worn) });
    }
    let Some(st) = w.entity(player) else { return };
    let dir = look(st.yaw, st.pitch);
    // From the eyes, a little ahead; the arrow's position is the bottom of its box.
    let eye = [st.pos[0], st.pos[1] + st.size[2] as f64, st.pos[2]];
    let at = [eye[0] + dir[0] * 0.5, eye[1] + dir[1] * 0.5 - 0.125, eye[2] + dir[2] * 0.5];
    let id = w.spawn(&ARROW, at);
    if let Some(a) = w.component_mut(id, &FLIGHT) {
        *a = Flight { shooter: player, damage: (DAMAGE * f).round().max(1.0), stuck: 0, age: 0, pickup: !creative as u32 };
    }
    if let Some(mut s) = w.entity(id) {
        let v = SPEED * f as f64;
        s.vel = [dir[0] * v, dir[1] * v, dir[2] * v];
        (s.yaw, s.pitch) = (st.yaw, st.pitch);
        w.set_entity(id, &s);
    }
}

/// The direction of a yaw and pitch (radians): yaw 0 looks to -z, pitch up is positive.
fn look(yaw: f32, pitch: f32) -> [f64; 3] {
    let (sy, cy) = (yaw as f64).sin_cos();
    let (sp, cp) = (pitch as f64).sin_cos();
    [-sy * cp, sp, -cy * cp]
}

/// Every arrow, every tick: on its way it falls and slows, hits the first block or creature on
/// the stretch it covers this tick; stuck, it waits to be picked up.
fn fly(t: &EntityTick) {
    let w = t.world();
    for &id in t.ids() {
        let (Some(mut st), Some(a)) = (w.entity(id), w.component(id, &FLIGHT).copied()) else { continue };
        let mut a = a;
        a.age += 1;
        if a.age > LIFETIME {
            w.remove(id);
            continue;
        }
        if a.stuck != 0 {
            if a.pickup != 0 && pick_up(&w, id, st.pos) {
                continue;
            }
            st.vel = [0.0; 3];
        } else {
            let mut v = st.vel;
            v[1] -= GRAVITY * TICK;
            v = v.map(|c| c * DRAG);
            // The middle of the arrow along this tick's stretch, a quarter block at a time.
            let from = [st.pos[0], st.pos[1] + 0.125, st.pos[2]];
            let length = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt() * TICK;
            let steps = (length / 0.25).ceil().max(1.0) as u32;
            let mut hit = false;
            for i in 1..=steps {
                let k = TICK * i as f64 / steps as f64;
                let p = [from[0] + v[0] * k, from[1] + v[1] * k, from[2] + v[2] * k];
                let cell = p.map(|c| c.floor() as i32);
                if w.flags(cell) & abi::block_flags::SOLID != 0 {
                    // Its middle at the last free point, so it takes the light there; the head,
                    // half a block ahead, is in the block.
                    let back = TICK * (i - 1) as f64 / steps as f64;
                    st.pos = [from[0] + v[0] * back, from[1] + v[1] * back - 0.125, from[2] + v[2] * back];
                    st.vel = [0.0; 3];
                    a.stuck = 1;
                    hit = true;
                    break;
                }
                // Anything alive whose box (a little bigger) it is in; not its shooter as it leaves.
                let target = w.near(p, 3.0, None).into_iter().find(|&e| {
                    e != id
                        && (e != a.shooter || a.age > 5)
                        && w.entity(e).is_some_and(|s| {
                            let r = s.size[0] as f64 / 2.0 + 0.15;
                            s.kind != ARROW.id()
                                && s.max_health > 0.0
                                && (p[0] - s.pos[0]).abs() <= r
                                && (p[2] - s.pos[2]).abs() <= r
                                && p[1] >= s.pos[1] - 0.15
                                && p[1] <= s.pos[1] + s.size[1] as f64 + 0.15
                        })
                });
                if let Some(target) = target {
                    if w.damage(target, a.damage, a.shooter, damage::ATTACK) {
                        let h = (v[0] * v[0] + v[2] * v[2]).sqrt().max(1e-6);
                        w.push(target, [v[0] / h * 4.0, 2.0, v[2] / h * 4.0]);
                    }
                    w.remove(id);
                    hit = true;
                    break;
                }
            }
            if a.stuck == 0 && !hit {
                st.vel = v;
                st.yaw = (-v[0]).atan2(-v[2]) as f32;
                st.pitch = v[1].atan2((v[0] * v[0] + v[2] * v[2]).sqrt()) as f32;
            }
        }
        w.set_entity(id, &st);
        if let Some(c) = w.component_mut(id, &FLIGHT) {
            *c = a;
        }
    }
}

/// A player close to a stuck arrow takes it back; true when it is gone.
fn pick_up(w: &World, id: u64, pos: [f64; 3]) -> bool {
    for p in w.near(pos, 2.0, Some(&PLAYER)) {
        if w.inventory(p).insert(ItemStack::new(ARROW_ITEM.id(), 1)).is_empty() {
            w.remove(id);
            return true;
        }
    }
    false
}

/// Since when the use key is held with the bow in hand, on this client.
static DRAWN: Mutex<Option<Instant>> = Mutex::new(None);

fn client(r: &mut Registry) {
    r.client_frame(|c| {
        let st = c.state();
        let holding = c.inventory_slot(st.selected).item == BOW.id();
        let drawing = holding && c.held(&keys::USE) && c.screen().is_empty();
        let mut drawn = DRAWN.lock().unwrap();
        match (*drawn, drawing) {
            (None, true) => *drawn = Some(Instant::now()),
            (Some(since), false) => {
                // Let go: shoot. Put away or a screen opened: only stop drawing.
                if holding && c.screen().is_empty() {
                    c.send(&SHOOT, &since.elapsed().as_secs_f32());
                }
                *drawn = None;
            }
            (Some(since), true) => {
                // How far it is drawn, under the crosshair.
                let step = ((power(since.elapsed().as_secs_f32()) * 4.0).ceil() as usize).clamp(1, 4) - 1;
                c.screen_text(0.5, 0.53, ["bow.base.charge.0", "bow.base.charge.1", "bow.base.charge.2", "bow.base.charge.3"][step], &[], 0xffffffff, 1.0);
            }
            (None, false) => {}
        }
    });
}
