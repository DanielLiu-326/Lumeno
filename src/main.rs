#![feature(normalize_lexically)]
#![recursion_limit = "256"]

use axum::extract::{DefaultBodyLimit, Path};
use axum::middleware;
use axum::routing::{patch, post};
use axum::{Extension, Router, routing::get};
use rand::Rng;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

rust_i18n::i18n!("locales", fallback = "en");

mod backend;
mod grade;

use backend::ai::{
    api_ai_duplicates, api_ai_index, api_ai_search_similar, api_ai_search_text, api_ai_status,
    api_album_face_crop, api_album_face_hints_get, api_album_faces_adopt_cluster,
    api_album_faces_delete, api_album_faces_get, api_album_faces_merge_clusters,
    api_album_faces_patch, api_album_faces_recluster, api_album_faces_remove_center,
    api_album_faces_split,
};
use backend::album::{
    api_albums_collections, api_albums_copy_photo, api_albums_create, api_albums_date_histogram,
    api_albums_delete, api_albums_delete_photos, api_albums_download, api_albums_download_file,
    api_albums_export, api_albums_export_batch, api_albums_export_batch_file, api_albums_import,
    api_albums_list, api_albums_photo_meta, api_albums_photos, api_albums_photos_query,
    api_albums_rebuild_index, api_albums_share, api_albums_unshare, api_albums_update,
    api_albums_upload, api_daily_picks, api_darktable_preview, api_dashboard, api_has_xmp,
    api_on_this_day, api_thumbnail, api_video_encode, api_video_file, api_video_info,
    spawn_thumbnail_prewarmer,
};
use backend::auth::{
    Role, SessionManager, UserManager, api_login, api_logout, api_register, api_whoami,
};
use backend::common::{
    AppState, ConcurrencyLimiter, RateLimiter, security_headers, spawn_cache_gc,
};
use backend::files::{
    api_copy_files, api_delete_files, api_download_file, api_get_file_list, api_move_files,
    api_new_folder, api_rename_file, api_upload, api_upload_feed, web_egui_file, web_file,
};
use backend::grade_api::{
    api_grade_full, api_grade_node, api_grade_open, api_grade_preview, api_grade_profile,
};
use backend::grade_resources::{
    api_grade_resources_delete, api_grade_resources_list, api_grade_resources_lut,
    api_grade_resources_preset, api_grade_resources_rename,
};
use backend::settings::{
    SettingsManager, api_admin_reset_password, api_admin_users_delete, api_admin_users_get,
    api_admin_users_update, api_avatar_get, api_avatar_upload, api_password_update,
    api_preferences_get, api_preferences_update, api_settings_get, api_settings_update,
    api_thumbnail_scan_now, api_wallpaper_get, api_wallpaper_upload, api_webdav_handler,
    api_webdav_shares_get, api_webdav_shares_update,
};
use backend::tasks::{
    TaskManager, api_cancel_task, api_clear_finished_tasks, api_get_task_list, api_get_task_status,
    api_pause_task, api_resume_task,
};

#[tokio::main]
async fn main() {
    let data_path: PathBuf = std::path::PathBuf::from("./dist/data")
        .canonicalize()
        .unwrap();
    let sessions_file = data_path.join("sessions.json");
    let settings_file = data_path.join("settings.json");
    let users_file = data_path.join("users.json");

    // Load or generate HMAC key
    let hmac_key_file = data_path.join("hmac_key.bin");
    let hmac_key = if hmac_key_file.exists() {
        std::fs::read(&hmac_key_file).expect("Failed to read HMAC key file")
    } else {
        use rand::RngCore;
        let mut key = vec![0u8; 32];
        rand::rng().fill_bytes(&mut key);
        std::fs::write(&hmac_key_file, &key).expect("Failed to write HMAC key file");
        eprintln!("Generated new HMAC key at {}", hmac_key_file.display());
        key
    };

    let trust_proxy_headers = std::env::var("TRUST_PROXY")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);

    let user_manager = Arc::new(UserManager::new(Some(users_file)));
    let settings_manager = Arc::new(SettingsManager::new(settings_file));
    let thumbnail_limiter = ConcurrencyLimiter::for_thumbnails(&settings_manager).await;
    let darktable_limiter = ConcurrencyLimiter::for_darktable(&settings_manager).await;
    let ai_limiter = ConcurrencyLimiter::for_ai(&settings_manager).await;
    backend::common::ensure_data_cache_layout(&data_path);
    let ai_service = backend::ai::AiService::new(&settings_manager, &data_path).await;
    let app_state = Arc::new(AppState {
        started_at: std::time::Instant::now(),
        data_path,
        resources_path: std::path::PathBuf::from("./dist/resources")
            .canonicalize()
            .unwrap(),
        egui_resources_path: std::path::PathBuf::from("./dist/resources-egui"),
        hmac_key,
        task_mgr: Arc::new(TaskManager::default()),
        user_manager: user_manager.clone(),
        session_manager: Arc::new(SessionManager::new(Some(sessions_file))),
        settings_manager,
        trust_proxy_headers,
        thumbnail_limiter,
        darktable_limiter,
        ai_limiter,
        ai_service,
    });

    // Bootstrap an admin user on first run.
    // Username defaults to admin. Password comes from BOOTSTRAP_ADMIN_PASSWORD,
    // or a one-time random value printed to stderr if that env is unset.
    if user_manager.is_empty().await {
        let admin_user =
            std::env::var("BOOTSTRAP_ADMIN_USER").unwrap_or_else(|_| "admin".to_string());
        let (admin_pass, generated) = match std::env::var("BOOTSTRAP_ADMIN_PASSWORD") {
            Ok(password) if !password.is_empty() => (password, false),
            _ => (generate_bootstrap_password(), true),
        };
        let passphrase = app_state.prehash_passphrase(&admin_pass);
        app_state
            .user_manager
            .new_user(&admin_user, &passphrase, Role::Admin, "en", "en")
            .await
            .unwrap();
        eprintln!("Bootstrap: created admin user '{admin_user}'");
        if generated {
            eprintln!("Bootstrap: generated password (shown once): {admin_pass}");
        }
    } else {
        eprintln!("Loaded users from users.json");
    }

    // Migration: legacy JSON sidecars → XMP
    {
        let sidecars = backend::album::scan_json_sidecars(&app_state.data_path).await;
        if !sidecars.is_empty() {
            eprintln!(
                "Found {} legacy .json sidecar(s). Migrate to XMP now? [y/N] ",
                sidecars.len()
            );
            let mut input = String::new();
            std::io::stdin()
                .read_line(&mut input)
                .expect("failed to read stdin");
            if input.trim().to_lowercase() != "y" {
                eprintln!("Aborted. Remove or migrate .json sidecars before starting.");
                std::process::exit(1);
            }
            eprintln!("Migrating {} sidecar(s)...", sidecars.len());
            for path in &sidecars {
                backend::album::migrate_json_sidecar(path).await;
            }
            eprintln!("Migration complete.");
        }
    }

    let index_html = "index.html".to_owned();
    let app = Router::new()
        .route("/api/list_files/{*path}", get(api_get_file_list))
        .route("/api/list_files/", get(api_get_file_list))
        .route("/api/thumbnail/{*path}", get(api_thumbnail))
        .route("/api/albums", get(api_albums_list).post(api_albums_create))
        .route("/api/albums/update", post(api_albums_update))
        .route("/api/albums/delete", post(api_albums_delete))
        .route("/api/albums/import", post(api_albums_import))
        .route("/api/albums/upload", post(api_albums_upload))
        .route(
            "/api/albums/photos",
            get(api_albums_photos).post(api_albums_photos_query),
        )
        .route("/api/albums/collections", get(api_albums_collections))
        .route("/api/albums/date_histogram", get(api_albums_date_histogram))
        .route("/api/albums/photo_meta", post(api_albums_photo_meta))
        .route("/api/albums/rebuild_index", post(api_albums_rebuild_index))
        .route("/api/albums/delete_photos", post(api_albums_delete_photos))
        .route("/api/albums/copy_photo", post(api_albums_copy_photo))
        .route("/api/albums/share", post(api_albums_share))
        .route("/api/albums/unshare", post(api_albums_unshare))
        .route("/api/albums/export", post(api_albums_export))
        .route("/api/albums/download", post(api_albums_download))
        .route("/api/albums/download/{id}", get(api_albums_download_file))
        .route("/api/albums/export_batch", post(api_albums_export_batch))
        .route(
            "/api/albums/export_batch/{id}",
            get(api_albums_export_batch_file),
        )
        .route(
            "/api/albums/darktable_preview/{*path}",
            get(api_darktable_preview),
        )
        .route("/api/grade/open", post(api_grade_open))
        .route("/api/grade/preview", post(api_grade_preview))
        .route("/api/grade/node", post(api_grade_node))
        .route("/api/grade/full", post(api_grade_full))
        .route("/api/grade/profile", post(api_grade_profile))
        .route("/api/grade/resources", get(api_grade_resources_list))
        .route(
            "/api/grade/resources/lut",
            post(api_grade_resources_lut)
                .layer(DefaultBodyLimit::max(256 * 1024 * 1024 + 1024 * 1024)),
        )
        .route(
            "/api/grade/resources/preset",
            post(api_grade_resources_preset),
        )
        .route(
            "/api/grade/resources/rename",
            post(api_grade_resources_rename),
        )
        .route(
            "/api/grade/resources/delete",
            post(api_grade_resources_delete),
        )
        .route("/api/albums/has_xmp/{*path}", get(api_has_xmp))
        .route("/api/albums/video_info/{*path}", get(api_video_info))
        .route("/api/albums/video_encode", post(api_video_encode))
        .route(
            "/api/albums/video_file/{codec}/{*path}",
            get(api_video_file),
        )
        .route("/api/on-this-day", get(api_on_this_day))
        .route("/api/daily-picks", get(api_daily_picks))
        .route("/api/dashboard", get(api_dashboard))
        .route("/api/ai/status", get(api_ai_status))
        .route("/api/ai/index", post(api_ai_index))
        .route("/api/ai/duplicates", get(api_ai_duplicates))
        .route("/api/ai/search/similar", post(api_ai_search_similar))
        .route("/api/ai/search/text", post(api_ai_search_text))
        .route("/api/ai/albums/{album}/faces", get(api_album_faces_get))
        .route(
            "/api/ai/albums/{album}/face-hints",
            get(api_album_face_hints_get),
        )
        .route(
            "/api/ai/albums/{album}/faces/recluster",
            post(api_album_faces_recluster),
        )
        .route(
            "/api/ai/albums/{album}/faces/adopt-cluster",
            post(api_album_faces_adopt_cluster),
        )
        .route(
            "/api/ai/albums/{album}/faces/merge-clusters",
            post(api_album_faces_merge_clusters),
        )
        .route(
            "/api/ai/albums/{album}/faces/split",
            post(api_album_faces_split),
        )
        .route(
            "/api/ai/albums/{album}/faces/remove-center",
            post(api_album_faces_remove_center),
        )
        .route(
            "/api/ai/albums/{album}/faces/{person_id}",
            patch(api_album_faces_patch).delete(api_album_faces_delete),
        )
        .route("/api/ai/albums/{album}/face-crop", get(api_album_face_crop))
        .route(
            "/api/settings",
            get(api_settings_get).post(api_settings_update),
        )
        .route("/api/settings/thumbnail-scan", post(api_thumbnail_scan_now))
        .route(
            "/api/preferences",
            get(api_preferences_get).post(api_preferences_update),
        )
        .route("/api/account/password", post(api_password_update))
        .route(
            "/api/account/avatar",
            post(api_avatar_upload).layer(DefaultBodyLimit::max(10 * 1024 * 1024)),
        )
        .route("/api/avatar/{filename}", get(api_avatar_get))
        .route(
            "/api/account/wallpaper",
            post(api_wallpaper_upload).layer(DefaultBodyLimit::max(15 * 1024 * 1024)),
        )
        .route("/api/wallpaper/{filename}", get(api_wallpaper_get))
        .route("/api/admin/users", get(api_admin_users_get))
        .route("/api/admin/users/update", post(api_admin_users_update))
        .route("/api/admin/users/delete", post(api_admin_users_delete))
        .route(
            "/api/admin/users/reset_password",
            post(api_admin_reset_password),
        )
        .route(
            "/api/webdav/shares",
            get(api_webdav_shares_get).post(api_webdav_shares_update),
        )
        .route("/dav/{*path}", axum::routing::any(api_webdav_handler))
        .route("/api/download/{*path}", get(api_download_file))
        .route("/api/download/", get(api_download_file))
        .route("/api/delete", post(api_delete_files))
        .route("/api/rename/{*path}", post(api_rename_file))
        .route("/api/new_folder/{*path}", get(api_new_folder))
        .route("/api/copy_files", post(api_copy_files))
        .route("/api/move_files", post(api_move_files))
        .route("/api/get_task_status/{*path}", get(api_get_task_status))
        .route("/api/cancel_task/{*path}", post(api_cancel_task))
        .route("/api/pause_task/{*path}", post(api_pause_task))
        .route("/api/resume_task/{*path}", post(api_resume_task))
        .route("/api/clear_finished_tasks", post(api_clear_finished_tasks))
        .route("/api/get_task_list/", get(api_get_task_list))
        .route("/api/upload", post(api_upload))
        .route(
            "/api/upload_feed",
            post(api_upload_feed).layer(DefaultBodyLimit::max(32 * 1024 * 1024)),
        )
        .route("/api/login", post(api_login))
        .route("/api/register", post(api_register))
        .route("/api/logout", post(api_logout))
        .route("/api/whoami", get(api_whoami))
        .route("/egui", get(|e| web_egui_file(None, e)))
        .route("/egui/{*path}", get(web_egui_file))
        .route("/{*path}", get(web_file))
        .route("/", get(|e| web_file(Some(Path(index_html)), e)))
        .layer(middleware::from_fn(security_headers))
        .layer(Extension(RateLimiter::new()))
        .layer(Extension(app_state.clone()));

    let listener = tokio::net::TcpListener::bind("[::]:3000").await.unwrap();
    print_startup_banner(&app_state);
    spawn_thumbnail_prewarmer(app_state.clone());
    spawn_cache_gc(app_state);
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .unwrap();
}

fn print_startup_banner(state: &AppState) {
    eprintln!(
        r#"
 ██╗     ██╗   ██╗███╗   ███╗███████╗███╗   ██╗ ██████╗
 ██║     ██║   ██║████╗ ████║██╔════╝████╗  ██║██╔═══██╗
 ██║     ██║   ██║██╔████╔██║█████╗  ██╔██╗ ██║██║   ██║
 ██║     ██║   ██║██║╚██╔╝██║██╔══╝  ██║╚██╗██║██║   ██║
 ███████╗╚██████╔╝██║ ╚═╝ ██║███████╗██║ ╚████║╚██████╔╝
 ╚══════╝ ╚═════╝ ╚═╝     ╚═╝╚══════╝╚═╝  ╚═══╝ ╚═════╝
"#
    );
    eprintln!("Lumeno {}", env!("CARGO_PKG_VERSION"));
    eprintln!("Listening   http://localhost:3000");
    eprintln!("Data        {}", state.data_path.display());
    eprintln!("Resources   {}", state.resources_path.display());
}

fn generate_bootstrap_password() -> String {
    const CHARSET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789";
    let mut rng = rand::rng();
    (0..20)
        .map(|_| {
            let idx = rng.random_range(0..CHARSET.len());
            CHARSET[idx] as char
        })
        .collect()
}
