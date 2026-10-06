use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex};
use winisland_plugin_api::abi::{
    ABI_VERSION_2, CAP_EVENTS, CAP_HOST_STATE, CAP_IMAGE, CAP_SURFACE, PluginCreateInfoV2,
    PluginDescriptorV2, PluginHandleV2, PluginStatus,
};
use winisland_plugin_api::sdk::{
    CallbackResource, DrawListBuilder, Error, Host, ImageHandle, Rgba, Size, Surface,
};
use winisland_plugin_api::{PluginMetadataC, PluginToken, SurfaceSpecV2, WidgetId};

const PLUGIN_ID: &str = "expanded-rotating-cd";
const PLUGIN_NAME: &str = "Expanded Rotating CD";
const PLUGIN_AUTHOR: &str = "phonkboisad";
const PLUGIN_DESCRIPTION: &str = "A spinning CD overlay for WinIsland's expanded music view";
const MUSIC_PAGE: u64 = 1;
const ROTATION_SECONDS: f64 = 2.5;
const EXPANDED_COVER_INSET: f32 = 24.0;
const EXPANDED_COVER_SIZE: f32 = 64.0;
const DISC_OVERHANG: f32 = 8.0;

struct RenderState {
    cover: Option<ImageHandle>,
    has_media: bool,
    playing: bool,
    expanded: bool,
    music_page: bool,
    angle: f32,
}

struct Instance {
    host: Host,
    surface: Option<Arc<Surface>>,
    state: Arc<Mutex<RenderState>>,
    state_events: Option<CallbackResource>,
    resize_events: Option<CallbackResource>,
}

static DESCRIPTOR: PluginDescriptorV2 = PluginDescriptorV2 {
    struct_size: std::mem::size_of::<PluginDescriptorV2>() as u32,
    abi_version: ABI_VERSION_2,
    capabilities: CAP_EVENTS | CAP_HOST_STATE | CAP_IMAGE | CAP_SURFACE,
    metadata: PluginMetadataC::new(
        PLUGIN_ID,
        PLUGIN_NAME,
        env!("CARGO_PKG_VERSION"),
        PLUGIN_AUTHOR,
        PLUGIN_DESCRIPTION,
    ),
    create: Some(create),
    shutdown: Some(shutdown),
    destroy: Some(destroy),
    on_tick: Some(on_tick),
};

unsafe extern "C" fn create(
    info: *const PluginCreateInfoV2,
    out_handle: *mut PluginHandleV2,
) -> PluginStatus {
    if info.is_null() || out_handle.is_null() {
        return PluginStatus::InvalidArgument;
    }
    // SAFETY: `out_handle` is a valid writable output pointer supplied by the host.
    unsafe { out_handle.write(std::ptr::null_mut()) };

    // SAFETY: The host supplies a readable create-info header for this callback.
    let info = unsafe { &*info };
    if info.struct_size < std::mem::size_of::<PluginCreateInfoV2>() as u32
        || info.abi_version != ABI_VERSION_2
        || info.plugin_token == PluginToken::INVALID
    {
        return PluginStatus::UnsupportedVersion;
    }

    // SAFETY: The host table remains valid for the lifetime of the plugin instance.
    let host = match unsafe { Host::from_raw(info.host_api, info.plugin_token) } {
        Ok(host) => host,
        Err(_) => return PluginStatus::InvalidArgument,
    };
    let log = host.log();
    match create_instance(host) {
        Ok(instance) => {
            // SAFETY: The opaque allocation remains alive until the host calls destroy.
            unsafe { out_handle.write(Box::into_raw(Box::new(instance)).cast::<c_void>()) };
            PluginStatus::Ok
        }
        Err(error) => {
            log.write(3, &format!("{PLUGIN_NAME} initialization failed: {error}"));
            PluginStatus::Internal
        }
    }
}

fn create_instance(host: Host) -> Result<Instance, Error> {
    let host_state = host.host_state()?.get()?;
    let events = host.events()?;
    let island = events.island_state()?;
    let has_media = host_state.media_title[0] != 0;
    let cover = if has_media { album_art(&host)? } else { None };
    let surface = Arc::new(host.surfaces()?.create(SurfaceSpecV2 {
        key: winisland_plugin_api::str_to_fixed("expanded-rotating-cd-overlay"),
        title: winisland_plugin_api::str_to_fixed(PLUGIN_NAME),
        kind: winisland_plugin_api::SURFACE_FOREGROUND,
        flags: winisland_plugin_api::SURFACE_ENABLED,
        width: 1.0,
        height: 1.0,
        ..Default::default()
    })?);
    let state = Arc::new(Mutex::new(RenderState {
        cover,
        has_media,
        playing: host_state.is_playing != 0,
        expanded: island.expanded != 0,
        music_page: island.page == MUSIC_PAGE,
        angle: 0.0,
    }));

    set_animation(&host, surface.id(), &state)?;

    let state_host = host.clone();
    let state_surface = Arc::clone(&surface);
    let state_render = Arc::clone(&state);
    let state_events = events.subscribe(
        winisland_plugin_api::EVENT_HOST | winisland_plugin_api::EVENT_MEDIA,
        WidgetId::INVALID,
        move |event| {
            let result = catch_unwind(AssertUnwindSafe(|| -> Result<(), Error> {
                let host_state = state_host.host_state()?.get()?;
                let island = state_host.events()?.island_state()?;
                let mut render = state_render
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                render.has_media = host_state.media_title[0] != 0;
                render.playing = host_state.is_playing != 0;
                render.expanded = island.expanded != 0;
                render.music_page = island.page == MUSIC_PAGE;
                if should_refresh_cover(event.kind) {
                    render.cover = if render.has_media {
                        album_art(&state_host)?
                    } else {
                        None
                    };
                }
                drop(render);
                set_animation(&state_host, state_surface.id(), &state_render)?;
                draw_overlay(&state_host, &state_surface, &state_render)
            }));
            log_callback_result(&state_host, "host/media update", result);
        },
    )?;

    let resize_host = host.clone();
    let resize_surface = Arc::clone(&surface);
    let resize_render = Arc::clone(&state);
    let resize_events = events.subscribe(
        winisland_plugin_api::EVENT_RESIZE,
        surface.id(),
        move |_| {
            let result = catch_unwind(AssertUnwindSafe(|| -> Result<(), Error> {
                let island = resize_host.events()?.island_state()?;
                {
                    let mut render = resize_render
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    render.expanded = island.expanded != 0;
                    render.music_page = island.page == MUSIC_PAGE;
                }
                set_animation(&resize_host, resize_surface.id(), &resize_render)?;
                draw_overlay(&resize_host, &resize_surface, &resize_render)
            }));
            log_callback_result(&resize_host, "resize", result);
        },
    )?;

    Ok(Instance {
        host,
        surface: Some(surface),
        state,
        state_events: Some(state_events),
        resize_events: Some(resize_events),
    })
}

fn log_callback_result(
    host: &Host,
    operation: &str,
    result: Result<Result<(), Error>, Box<dyn std::any::Any + Send>>,
) {
    match result {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            host.log()
                .write(3, &format!("{PLUGIN_NAME} {operation} failed: {error}"));
        }
        Err(_) => {
            host.log().write(
                3,
                &format!("{PLUGIN_NAME} {operation} panicked; update was ignored"),
            );
        }
    }
}

fn album_art(host: &Host) -> Result<Option<ImageHandle>, Error> {
    match host.images()?.album_art() {
        Ok(cover) => Ok(Some(cover)),
        Err(Error::Io) => Ok(None),
        Err(error) => Err(error),
    }
}

fn should_refresh_cover(event_kind: u64) -> bool {
    event_kind & (winisland_plugin_api::EVENT_HOST | winisland_plugin_api::EVENT_MEDIA) != 0
}

fn set_animation(host: &Host, target: WidgetId, state: &Mutex<RenderState>) -> Result<(), Error> {
    let render = state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    host.events()?.set_animation(
        target,
        render.has_media
            && render.cover.is_some()
            && render.playing
            && render.expanded
            && render.music_page,
    )
}

fn draw_overlay(host: &Host, surface: &Surface, state: &Mutex<RenderState>) -> Result<(), Error> {
    let (width, height) = surface.logical_size();
    let island = host.events()?.island_state()?;
    let state = state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut frame = DrawListBuilder::new(Size::new(width, height));
    if should_draw(
        state.has_media,
        state.expanded,
        state.music_page,
        state.cover.is_some(),
    ) && let Some(cover) = state.cover.as_ref()
        && let Some((x, y, diameter)) = disc_geometry(width, height, island.width, island.scale)
    {
        draw_disc(&mut frame, cover, x, y, diameter, state.angle);
    }
    surface.submit(frame.finish())
}

fn disc_geometry(
    surface_width: f32,
    surface_height: f32,
    island_width: f32,
    expanded_scale: f32,
) -> Option<(f32, f32, f32)> {
    if !surface_width.is_finite()
        || !surface_height.is_finite()
        || !island_width.is_finite()
        || !expanded_scale.is_finite()
        || surface_width <= 0.0
        || surface_height <= 0.0
        || island_width <= 0.0
        || expanded_scale <= 0.0
    {
        return None;
    }
    let expanded_to_surface = expanded_scale * surface_width / island_width;
    let diameter = (EXPANDED_COVER_SIZE + DISC_OVERHANG * 2.0) * expanded_to_surface;
    if diameter > surface_height {
        return None;
    }
    let inset = (EXPANDED_COVER_INSET - DISC_OVERHANG) * expanded_to_surface;
    Some((inset, inset, diameter))
}

fn draw_disc(
    frame: &mut DrawListBuilder,
    cover: &ImageHandle,
    x: f32,
    y: f32,
    diameter: f32,
    angle: f32,
) {
    let radius = diameter * 0.5;
    let center_x = x + radius;
    let center_y = y + radius;
    let bounds = winisland_plugin_api::sdk::Rect::new(x, y, diameter, diameter);
    let radians = angle.to_radians();
    let (sin, cos) = radians.sin_cos();

    frame.fill_circle(center_x, center_y, radius, Rgba::from_argb(0xff08_0a0d));
    frame.clip_round_rect(bounds, radius);
    frame.transform([
        cos,
        sin,
        -sin,
        cos,
        center_x - cos * center_x + sin * center_y,
        center_y - sin * center_x - cos * center_y,
    ]);
    let image_diameter = diameter * std::f32::consts::SQRT_2;
    frame.image(
        cover.id(),
        winisland_plugin_api::sdk::Rect::new(
            center_x - image_diameter * 0.5,
            center_y - image_diameter * 0.5,
            image_diameter,
            image_diameter,
        ),
        winisland_plugin_api::sdk::ImageFit::Cover,
    );

    for groove_radius in [radius * 0.67, radius * 0.84] {
        let groove_bounds = winisland_plugin_api::sdk::Rect::new(
            center_x - groove_radius,
            center_y - groove_radius,
            groove_radius * 2.0,
            groove_radius * 2.0,
        );
        frame.stroke_arc(
            groove_bounds,
            winisland_plugin_api::sdk::Deg12(0.0),
            winisland_plugin_api::sdk::Deg12(360.0),
            0.4,
            Rgba::from_argb(0x40ff_ffff),
        );
    }

    frame.pop_transform();
    frame.pop_clip();

    frame.stroke_arc(
        bounds,
        winisland_plugin_api::sdk::Deg12(0.0),
        winisland_plugin_api::sdk::Deg12(360.0),
        (diameter * 0.012).max(0.8),
        Rgba::from_argb(0xdddd_ffff),
    );
}

unsafe extern "C" fn on_tick(
    handle: PluginHandleV2,
    target: WidgetId,
    dt_seconds: f64,
) -> PluginStatus {
    if handle.is_null() || !dt_seconds.is_finite() || dt_seconds < 0.0 {
        return PluginStatus::InvalidArgument;
    }
    if dt_seconds == 0.0 {
        return PluginStatus::Ok;
    }
    // SAFETY: The host only calls on_tick for an instance allocated by create.
    let instance = unsafe { &mut *handle.cast::<Instance>() };
    let Some(surface) = instance.surface.as_ref() else {
        return PluginStatus::StaleHandle;
    };
    if target != surface.id() {
        return PluginStatus::Ok;
    }

    let result = catch_unwind(AssertUnwindSafe(|| -> Result<(), Error> {
        let mut state = instance
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !state.has_media
            || state.cover.is_none()
            || !state.playing
            || !state.expanded
            || !state.music_page
        {
            return Ok(());
        }
        state.angle = advance_angle(state.angle, dt_seconds);
        drop(state);
        draw_overlay(&instance.host, surface, &instance.state)
    }));
    match result {
        Ok(Ok(())) => PluginStatus::Ok,
        Ok(Err(error)) => {
            instance
                .host
                .log()
                .write(3, &format!("{PLUGIN_NAME} frame failed: {error}"));
            PluginStatus::Internal
        }
        Err(_) => {
            instance.host.log().write(
                3,
                &format!("{PLUGIN_NAME} frame panicked; frame was ignored"),
            );
            PluginStatus::Internal
        }
    }
}

fn advance_angle(angle: f32, dt_seconds: f64) -> f32 {
    (angle + (dt_seconds.rem_euclid(ROTATION_SECONDS) * 360.0 / ROTATION_SECONDS) as f32)
        .rem_euclid(360.0)
}

fn should_draw(has_media: bool, expanded: bool, music_page: bool, has_cover: bool) -> bool {
    has_media && expanded && music_page && has_cover
}

unsafe extern "C" fn shutdown(handle: PluginHandleV2) -> PluginStatus {
    if handle.is_null() {
        return PluginStatus::InvalidArgument;
    }
    // SAFETY: The host calls shutdown before destroy for this live instance.
    let instance = unsafe { &mut *handle.cast::<Instance>() };
    for callbacks in [&mut instance.state_events, &mut instance.resize_events] {
        if let Some(callback) = callbacks.as_mut()
            && let Err(error) = callback.cancel()
        {
            instance.host.log().write(
                3,
                &format!("{PLUGIN_NAME} callback shutdown failed: {error}"),
            );
            return PluginStatus::Internal;
        }
    }
    drop(instance.state_events.take());
    drop(instance.resize_events.take());
    drop(instance.surface.take());
    let mut state = instance
        .state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    drop(state.cover.take());
    PluginStatus::Ok
}

unsafe extern "C" fn destroy(handle: PluginHandleV2) {
    if !handle.is_null() {
        // SAFETY: The host calls destroy once after successful shutdown.
        unsafe { drop(Box::from_raw(handle.cast::<Instance>())) };
    }
}

/// # Safety
/// WinIsland calls this exported function using the ABI v2 entry signature.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn winisland_plugin_entry_v2() -> *const PluginDescriptorV2 {
    &DESCRIPTOR
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_uses_abi_v2_and_distinct_package_id() {
        assert_eq!(DESCRIPTOR.abi_version, ABI_VERSION_2);
        assert_eq!(
            &DESCRIPTOR.metadata.id[..PLUGIN_ID.len()],
            PLUGIN_ID.as_bytes()
        );
        assert_eq!(
            &DESCRIPTOR.metadata.description[..PLUGIN_DESCRIPTION.len()],
            PLUGIN_DESCRIPTION.as_bytes()
        );
        assert_eq!(DESCRIPTOR.capabilities & CAP_EVENTS, CAP_EVENTS);
        assert_eq!(DESCRIPTOR.capabilities & CAP_IMAGE, CAP_IMAGE);
        assert_eq!(DESCRIPTOR.capabilities & CAP_SURFACE, CAP_SURFACE);
    }

    #[test]
    fn disc_only_appears_in_expanded_music_mode() {
        assert!(should_draw(true, true, true, true));
        assert!(!should_draw(true, true, false, true));
        assert!(!should_draw(true, false, true, true));
        assert!(!should_draw(false, true, true, true));
    }

    #[test]
    fn disc_geometry_scales_and_stays_inside_expanded_surface() {
        assert_eq!(
            disc_geometry(1000.0, 500.0, 800.0, 1.6),
            Some((32.0, 32.0, 160.0))
        );
        assert_eq!(disc_geometry(1000.0, 40.0, 800.0, 1.6), None);
        assert_eq!(disc_geometry(1000.0, 500.0, 0.0, 1.6), None);
    }

    #[test]
    fn rotation_advances_and_wraps() {
        assert_eq!(advance_angle(0.0, 0.5), 72.0);
        assert_eq!(advance_angle(0.0, ROTATION_SECONDS), 0.0);
        assert_eq!(advance_angle(359.0, 2.0), 287.0);
    }
}
