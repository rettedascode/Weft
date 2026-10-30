//! The test list. Each test is an `async fn(T) -> R`; failed checks are collected, `?` stops it.

mod bodies;
mod extend;
mod gameplay;
mod liquids;
mod looks;
mod probe;
mod scene;
mod sync;
mod ui;
mod visual;
mod world;

use std::time::Duration;

use crate::runner::{R, T, TestDef};

pub fn add(p: [i32; 3], d: [i32; 3]) -> [i32; 3] {
    [p[0] + d[0], p[1] + d[1], p[2] + d[2]]
}

pub fn at(p: [i32; 3]) -> String {
    format!("{} {} {}", p[0], p[1], p[2])
}

pub fn all() -> Vec<TestDef> {
    crate::tests![
        scene::scene,
        ui::screens,
        ui::scrollbar,
        ui::rebind,
        ui::f3,
        ui::pack_drag,
        ui::hud_editor,
        ui::debug_lines,
        ui::inventory_crafting,
        ui::creative_inventory,
        ui::chat,
        ui::mod_settings,
        ui::recipe_viewer,
        ui::inventory_hotkeys,
        world::persistence,
        world::missing_dimension,
        world::dimensions,
        world::virtual_menus,
        world::worldgen,
        world::plants_on_cube_tops,
        world::foreign_terrain,
        world::tick_control,
        world::growth,
        world::catch_up,
        world::dynamite,
        world::crater_saved,
        world::crater_view,
        world::sky_column_edit,
        world::edit_while_loading,
        world::gpu_terrain,
        world::distant_terrain,
        gameplay::crafting_table_tags,
        gameplay::sprint_and_sneak_modes,
        gameplay::mining_and_wear,
        gameplay::creative_and_invulnerable,
        gameplay::creative_clicks,
        gameplay::armor,
        gameplay::slot_accepts,
        gameplay::slot_limit,
        gameplay::bucket,
        gameplay::bow,
        gameplay::battery,
        gameplay::burning,
        gameplay::entity_fields,
        gameplay::fall_damage,
        gameplay::sneak_edge,
        gameplay::movement_attributes,
        gameplay::mob_physics,
        gameplay::player_box,
        gameplay::own_names,
        gameplay::food,
        gameplay::drop_and_pickup,
        gameplay::sounds,
        extend::events_batched,
        extend::vetoes_and_timers,
        extend::names,
        extend::key_to_server,
        extend::typed_channels,
        extend::block_properties,
        extend::client_entity_tick,
        extend::facing_placement,
        liquids::mod_liquid_bucket,
        liquids::mod_sand_soil,
        sync::menu_sync,
        visual::held_items,
        visual::hide_hud,
        visual::liquid_layers,
        visual::break_overlay,
        visual::use_swing,
        visual::inventory_figure,
        visual::tooltips,
        visual::night_light,
        visual::third_person,
        visual::player_skin,
        visual::ladder_placement,
        visual::wall_torch,
        visual::crouch_pose,
        visual::pick_block,
        visual::gizmos,
        visual::multiplayer,
        visual::client_hooks,
        looks::particles,
        looks::shaders_and_glow,
        looks::texture_seams,
        looks::texture_reload,
        looks::pack_lang,
        looks::tint_kinds,
        looks::broken_shader_pack,
        looks::rich_text,
        looks::gui_shader,
        looks::models,
        looks::block_model_file,
        looks::item_model_file,
        bodies::lift,
        bodies::ride_frames,
        bodies::skiff,
        bodies::skiff_smooth,
        bodies::skiff_pause,
        gameplay::respawn,
        bodies::skiff_blocks,
        bodies::natural_spawning,
        bodies::zombie_chases,
        bodies::morph,
    ]
}

/// Measurements without a verdict worth failing a run over; `--test bench`.
pub fn bench() -> Vec<TestDef> {
    crate::tests![world::load_benchmark, world::item_farm, looks::particle_crowd, world::crater]
}

impl T {
    /// A fresh world moved to the flat test dimension, survival, at noon, loaded and idle.
    pub async fn flat(&self) -> R {
        self.world(1).await?;
        self.cmd("/dim base:flatlands").await?;
        self.until("flatlands", Duration::from_secs(20), |t| t.state().dim != 0 || t.state().pos[1] < 10.0).await?;
        self.loaded().await?;
        self.op("mode survival").await?;
        self.idle().await
    }

    /// Turns the local player to look at the middle of block `p` from its eyes.
    pub fn look_at(&self, p: [i32; 3]) {
        let s = self.state();
        let eye = [s.pos[0], s.pos[1] + self.c().entity(0).map_or(1.62, |e| e.size[2] as f64), s.pos[2]];
        let d = [p[0] as f64 + 0.5 - eye[0], p[1] as f64 + 0.5 - eye[1], p[2] as f64 + 0.5 - eye[2]];
        let yaw = (-d[0]).atan2(-d[2]).to_degrees() as f32;
        let pitch = d[1].atan2((d[0] * d[0] + d[2] * d[2]).sqrt()).to_degrees() as f32;
        self.c().look(yaw, pitch);
    }

    /// Counts entities of a kind near the player as the server has them.
    pub async fn count_near(&self, kind: &str, r: f64) -> Result<usize, String> {
        let list = self.op(&format!("near ~ ~ ~ {r}")).await?;
        Ok(list.split_whitespace().filter(|e| e.ends_with(&format!(":{kind}"))).count())
    }
}
