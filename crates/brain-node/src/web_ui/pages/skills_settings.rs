use crate::web_ui::{render, AppResult, AppState};
use axum::{extract::State, response::Html};
use minijinja::context;

pub async fn handler(State(state): State<AppState>) -> AppResult<Html<String>> {
    let cfg = state.skill_config.read().await.clone();
    render(
        &state,
        "settings_skills.html",
        context! {
            active => "settings_skills",
            latitude => cfg.latitude,
            longitude => cfg.longitude,
            location_display_name => cfg.location_display_name,
            weather_api_base => cfg.weather_api_base,
            home_assistant_url => cfg.home_assistant_url.unwrap_or_default(),
            home_assistant_token => cfg.home_assistant_token.unwrap_or_default(),
            alsa_control => cfg.alsa_control,
            volume_step_pct => cfg.volume_step_pct,
            navidrome_url => cfg.navidrome_url.unwrap_or_default(),
            navidrome_user => cfg.navidrome_user.unwrap_or_default(),
            navidrome_password => cfg.navidrome_password.unwrap_or_default(),
            update_check_enabled => cfg.update_check_enabled,
        },
    )
}
