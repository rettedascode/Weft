//! The bow: held down with the use key it draws, let go it shoots an arrow from the inventory,
//! harder the longer it was drawn (a second for full power). While drawn, the bow in hand is one
//! of three variants with the string further back, which everyone sees. Arrows fly on their own
//! path with gravity and drag and a faint trail, hurt what they hit and stick in blocks, where
//! their shooter can pick them up again. Flint for their heads comes from gravel.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

use weft_sdk::*;

pub static BOW: Item = Item::named("bow");
/// The bow drawn a little, more, and fully: what the bow in hand turns into while drawn.
static PULLING: [Item; 3] = [Item::named("bow_pulling_0"), Item::named("bow_pulling_1"), Item::named("bow_pulling_2")];
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
/// Client to server: how far the bow is drawn, 1 to 3 (`PULLING`), 0 when it no longer is. Sent
/// when it changes and once a second while drawn.
static DRAW: Channel<u32> = Channel::named("bow_draw");
/// Server: players drawing, with when they last said so. A drawn bow of anyone else (they
/// dropped it, left, the client went quiet) turns back into a plain bow.
static DRAWING: Mutex<Option<HashMap<u64, Instant>>> = Mutex::new(None);
static TRAIL: Particle = Particle::named("arrow_trail");

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

/// Which of `PULLING` shows at `power` (1..3), vanilla's steps.
fn stage(power: f32) -> u32 {
    if power >= 0.9 {
        3
    } else if power >= 0.65 {
        2
    } else {
        1
    }
}

/// The bow or one of its drawn variants.
fn is_bow(item: u32) -> bool {
    item == BOW.id() || PULLING.iter().any(|p| p.id() == item)
}

pub fn register(r: &mut Registry) {
    r.item(&BOW).max_stack(1).durability(384).tab(&crate::tabs::COMBAT);
    for p in &PULLING {
        r.item(p).max_stack(1).durability(384).variant_of(&BOW);
    }
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
    r.packet(&DRAW, |p, stage| draw(&p.world(), p.player(), stage));
    r.world_tick(straighten_bows);
    r.particle(&TRAIL).textures(&["trail"]).life(0.3, 0.45).size(0.06, 0.1).gravity(0.0).drag(0.85).color(0xC9CED8B0, 0xC9CED800).translucent();
    client(r);
}

/// The held bow as drawn to `stage` (0: a plain bow again), its wear kept.
fn draw(w: &World, player: u64, stage: u32) {
    let hotbar = w.inventory(player).group("hotbar");
    let slot = w.selected_slot(player);
    let held = hotbar.get(slot);
    if !is_bow(held.item) {
        return;
    }
    let item = match stage {
        1..=3 => PULLING[stage as usize - 1].id(),
        _ => BOW.id(),
    };
    if held.item != item {
        hotbar.set(slot, ItemStack { item, ..held });
    }
    let mut drawing = DRAWING.lock().unwrap();
    let drawing = drawing.get_or_insert_with(HashMap::new);
    if stage == 0 {
        drawing.remove(&player);
    } else {
        drawing.insert(player, Instant::now());
    }
}

/// Twice a second: a drawn bow that nobody is drawing (dropped and picked up, its player gone
/// quiet) turns back into a plain one.
fn straighten_bows(w: &World) {
    for id in w.players() {
        if w.entity(id).is_none_or(|e| e.age % 10 != 0) {
            continue;
        }
        let drawing = DRAWING.lock().unwrap().as_ref().and_then(|d| d.get(&id).copied()).is_some_and(|t| t.elapsed().as_secs_f32() < 2.5);
        let inv = w.inventory(id);
        let selected = w.selected_slot(id);
        for i in 0..inv.size() {
            let s = inv.get(i);
            if s.item != BOW.id() && is_bow(s.item) && !(drawing && i == selected) {
                inv.set(i, ItemStack { item: BOW.id(), ..s });
            }
        }
    }
}

/// Shoots one arrow from `player`'s bow, drawn for `secs`.
fn shoot(w: &World, player: u64, secs: f32) {
    let f = power(secs);
    if f < 0.1 {
        return;
    }
    let hotbar = w.inventory(player).group("hotbar");
    let slot = w.selected_slot(player);
    // Back to a plain bow, whatever happens next.
    let held = hotbar.get(slot);
    if !is_bow(held.item) {
        return;
    }
    if let Some(d) = DRAWING.lock().unwrap().as_mut() {
        d.remove(&player);
    }
    let bow = ItemStack { item: BOW.id(), ..held };
    hotbar.set(slot, bow);
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

/// On this client: since when the use key is held with the bow in hand, and the stage last told
/// the server with when.
static DRAWN: Mutex<Option<(Instant, u32, Instant)>> = Mutex::new(None);
/// Where each flying arrow was last frame, for its trail.
static LAST_SEEN: Mutex<Option<HashMap<u64, [f64; 3]>>> = Mutex::new(None);

fn client(r: &mut Registry) {
    r.client_frame(|c| {
        let st = c.state();
        let holding = is_bow(c.inventory_slot(st.selected).item);
        let drawing = holding && c.held(&keys::USE) && c.screen().is_empty();
        let mut drawn = DRAWN.lock().unwrap();
        match (*drawn, drawing) {
            (None, true) => {
                c.send(&DRAW, &1);
                *drawn = Some((Instant::now(), 1, Instant::now()));
            }
            (Some((since, _, _)), false) => {
                // Let go: shoot. Put away or a screen opened: only stop drawing.
                if holding && c.screen().is_empty() {
                    c.send(&SHOOT, &since.elapsed().as_secs_f32());
                } else {
                    c.send(&DRAW, &0);
                }
                *drawn = None;
            }
            (Some((since, told, at)), true) => {
                let p = power(since.elapsed().as_secs_f32());
                let now = stage(p);
                if now != told || at.elapsed().as_secs_f32() > 1.0 {
                    c.send(&DRAW, &now);
                    *drawn = Some((since, now, Instant::now()));
                }
                // How far it is drawn, under the crosshair.
                let step = ((p * 4.0).ceil() as usize).clamp(1, 4) - 1;
                c.screen_text(0.5, 0.53, ["bow.base.charge.0", "bow.base.charge.1", "bow.base.charge.2", "bow.base.charge.3"][step], &[], 0xffffffff, 1.0);
            }
            (None, false) => {}
        }
    });
    // A faint trail behind arrows in flight, made by each client for itself.
    r.client_entity_tick(&ARROW, |c, e| {
        let mut seen = LAST_SEEN.lock().unwrap();
        let seen = seen.get_or_insert_with(HashMap::new);
        if seen.len() > 512 {
            seen.clear();
        }
        let pos = e.pos();
        let mid = [pos[0], pos[1] + 0.125, pos[2]];
        if let Some(was) = seen.insert(e.id(), mid) {
            let d = [mid[0] - was[0], mid[1] - was[1], mid[2] - was[2]];
            let moved = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
            // A puff every fifth of a block it moved; none once it is stuck.
            if moved > 0.05 {
                let n = (moved / 0.2).ceil().min(16.0) as u32;
                for i in 0..n {
                    let k = i as f64 / n as f64;
                    drop(c.spawn_particles(&TRAIL, [was[0] + d[0] * k, was[1] + d[1] * k, was[2] + d[2] * k]));
                }
            }
        }
        if e.dying().is_some_and(|d| d >= 1.0) {
            seen.remove(&e.id());
        }
    });
}
