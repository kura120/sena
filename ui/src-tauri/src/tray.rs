use tauri::image::Image;
use tauri::menu::{MenuBuilder, MenuItemBuilder};
use tauri::tray::TrayIconBuilder;
use tracing::{error, info};

/// Setup the system tray with Open and Quit menu items
pub fn setup_system_tray(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    info!("Setting up system tray");

    // Build the tray menu
    let open_item = MenuItemBuilder::with_id("open", "Open Debug Overlay").build(app)?;
    let quit_item = MenuItemBuilder::with_id("quit", "Quit").build(app)?;
    let menu = MenuBuilder::new(app)
        .item(&open_item)
        .separator()
        .item(&quit_item)
        .build()?;

    // Load the tray icon — try from icons directory, fallback to embedded default
    let icon = load_tray_icon(app)?;

    // Build and register the tray icon
    TrayIconBuilder::new()
        .icon(icon)
        .menu(&menu)
        .tooltip("Sena Debug Overlay")
        .on_menu_event(|app_handle, event| {
            match event.id().as_ref() {
                "open" => {
                    if let Err(e) = crate::overlay::show_all_panels(app_handle) {
                        error!(error = %e, "Failed to open overlay from tray");
                    }
                }
                "quit" => {
                    info!("Quit requested from system tray");
                    app_handle.exit(0);
                }
                _ => {}
            }
        })
        .build(app)?;

    info!("System tray setup complete");
    Ok(())
}

/// Load the tray icon from the icons directory or use default
fn load_tray_icon(_app: &tauri::App) -> Result<Image<'_>, Box<dyn std::error::Error>> {
    // Use the embedded 32x32 icon from the icons directory
    let icon_bytes = include_bytes!("../icons/32x32.png");
    
    // Decode the PNG to get RGBA data
    let img = image::load_from_memory(icon_bytes)
        .map_err(|e| format!("Failed to decode icon: {}", e))?;
    let rgba = img.to_rgba8();
    let (width, height) = rgba.dimensions();
    
    Ok(Image::new_owned(rgba.into_raw(), width, height))
}
