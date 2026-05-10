pub fn build() -> minijinja::Environment<'static> {
    let mut env = minijinja::Environment::new();
    env.add_template("base.html", include_str!("templates/base.html"))
        .unwrap();
    env.add_template("dashboard.html", include_str!("templates/dashboard.html"))
        .unwrap();
    env.add_template("nodes.html", include_str!("templates/nodes.html"))
        .unwrap();
    env.add_template("nodes_pair.html", include_str!("templates/nodes_pair.html"))
        .unwrap();
    env.add_template("documents.html", include_str!("templates/documents.html"))
        .unwrap();
    env.add_template("skills.html", include_str!("templates/skills.html"))
        .unwrap();
    env.add_template(
        "settings_tts.html",
        include_str!("templates/settings_tts.html"),
    )
    .unwrap();
    env.add_template(
        "settings_models.html",
        include_str!("templates/settings_models.html"),
    )
    .unwrap();
    env.add_template(
        "training_wake_word.html",
        include_str!("templates/training_wake_word.html"),
    )
    .unwrap();
    env.add_template(
        "training_voice.html",
        include_str!("templates/training_voice.html"),
    )
    .unwrap();
    env.add_template("setup.html", include_str!("templates/setup.html"))
        .unwrap();
    env
}
