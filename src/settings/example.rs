use super::*;

const EXAMPLE_COMMENT_COL: usize = 37;

const TOML_EXAMPLE_HEADER: &str = "# パスの相対パスは、書いた値も既定値も、この設定ファイルのあるディレクトリを起点にする（環境変数で渡したパスはカレントディレクトリが起点）\n";

fn push_padded_comment(out: &mut String, assignment: &str, note: &str) {
    let len = assignment.chars().count();
    out.push_str(assignment);
    if len < EXAMPLE_COMMENT_COL {
        out.push_str(&" ".repeat(EXAMPLE_COMMENT_COL - len));
    } else {
        out.push('\n');
        out.push_str(&" ".repeat(EXAMPLE_COMMENT_COL));
    }
    out.push_str("# ");
    out.push_str(note);
    out.push('\n');
}

pub fn render_toml_example() -> String {
    let mut out = String::from(TOML_EXAMPLE_HEADER);
    for section in SECTION_ORDER {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&format!("[{section}]\n"));
        for s in SETTINGS.iter().filter(|s| s.section == section) {
            let literal = s.example.literal();
            let assignment = if s.example.is_active() {
                format!("{} = {}", s.field, literal)
            } else {
                format!("#{} = {}", s.field, literal)
            };
            let note = format!("{}: {}", s.env, s.description.ja);
            push_padded_comment(&mut out, &assignment, &note);
        }
    }
    out
}

fn strip_toml_string(literal: &str) -> String {
    literal.trim().trim_matches('"').to_string()
}

fn env_default_literal(kind: Kind, literal: &str) -> String {
    match kind {
        Kind::List => literal
            .trim()
            .trim_start_matches('[')
            .trim_end_matches(']')
            .split(',')
            .map(|s| s.trim().trim_matches('"'))
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(","),
        _ => strip_toml_string(literal),
    }
}

/// Lives here rather than in the catalog because it's docker-compose wiring knowledge (which env vars `compose.yaml` fixes directly for the mirror service), not a per-setting fact.
const ENV_NO_EFFECT_IN_COMPOSE: &[&str] = &[
    "ipfs.api",
    "agent.state_dir",
    "kubo.managed",
    "gateway.upstream",
];

const ENV_EXAMPLE_SECRET_KEY_HEADER: &str = "# nsec1... または hex 形式の秘密鍵。サイト公開専用の鍵を新しく作って使うことを推奨する。\n# 空のままでもコンテナは起動する（ダッシュボードだけが動くセットアップモードになり、ブラウザのセットアップ画面から鍵を生成・保存できる。docs/architecture/up.md 参照）\nSWING_NOSTR_SECRET_KEY=\n";

const ENV_EXAMPLE_COMPOSE_EPILOGUE: &str = "# --- Docker Compose 専用のホストバインド（上の SWING_* とは別物。config::env_var は読まない） ---\n# Kubo のゲートウェイをホストのどこに公開するか（ipfs コンテナ）\n#SWING_KUBO_GATEWAY_BIND=127.0.0.1:8080\n# swing 内蔵ゲートウェイをホストのどこに公開するか（mirror コンテナ）\n#SWING_GATEWAY_BIND=127.0.0.1:8081\n# ダッシュボードをホストのどこに公開するか（mirror コンテナ）\n#SWING_DASHBOARD_BIND=127.0.0.1:8082\n";

pub fn render_env_example() -> String {
    let mut out = String::new();
    out.push_str(ENV_EXAMPLE_SECRET_KEY_HEADER);
    for section in SECTION_ORDER {
        let mut settings = SETTINGS
            .iter()
            .filter(|s| s.section == section && s.key != "nostr.secret_key")
            .peekable();
        if settings.peek().is_none() {
            continue;
        }
        out.push_str(&format!("\n# [{section}]\n"));
        for s in settings {
            let default = env_default_literal(s.kind, s.example.literal());
            let note = if ENV_NO_EFFECT_IN_COMPOSE.contains(&s.key) {
                format!(
                    "{}（compose の mirror コンテナでは compose.yaml が固定で渡すため、ここに書いても効果が無い）",
                    s.description.ja
                )
            } else {
                s.description.ja.to_string()
            };
            out.push_str(&format!("# {note}\n"));
            out.push_str(&format!("#{}={default}\n", s.env));
        }
    }
    out.push('\n');
    out.push_str(ENV_EXAMPLE_COMPOSE_EPILOGUE);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swing_example_toml_matches_generator() {
        let generated = render_toml_example();
        let checked_in = include_str!("../../swing.example.toml");
        assert_eq!(
            checked_in, generated,
            "swing.example.toml is out of date; run `swing config example > swing.example.toml` and commit the result"
        );
    }

    #[test]
    fn env_example_matches_generator() {
        let generated = render_env_example();
        let checked_in = include_str!("../../.env.example");
        assert_eq!(
            checked_in, generated,
            ".env.example is out of date; run `swing config env-example > .env.example` and commit the result"
        );
    }

    #[test]
    fn env_no_effect_in_compose_matches_compose_yaml() {
        fn indent(line: &str) -> usize {
            line.len() - line.trim_start().len()
        }

        fn find_after(lines: &[&str], start: usize, end: usize, target: &str) -> Option<usize> {
            lines[start..end]
                .iter()
                .position(|l| l.trim() == target)
                .map(|i| start + i)
        }

        fn block_end(lines: &[&str], header: usize, limit: usize) -> usize {
            let header_indent = indent(lines[header]);
            lines[header + 1..limit]
                .iter()
                .position(|l| !l.trim().is_empty() && indent(l) <= header_indent)
                .map(|i| header + 1 + i)
                .unwrap_or(limit)
        }

        fn mirror_environment_literal_swing_vars(compose: &str) -> Vec<&str> {
            let lines: Vec<&str> = compose.lines().collect();
            let mirror = find_after(&lines, 0, lines.len(), "mirror:")
                .expect("compose.yaml: no `mirror:` service found");
            let mirror_end = block_end(&lines, mirror, lines.len());
            let environment = find_after(&lines, mirror, mirror_end, "environment:")
                .expect("compose.yaml: mirror service has no `environment:` block");
            let environment_end = block_end(&lines, environment, mirror_end);

            lines[environment + 1..environment_end]
                .iter()
                .filter_map(|line| {
                    let entry = line.trim().trim_start_matches("- ");
                    let (key, value) = entry.split_once([':', '='])?;
                    let key = key.trim();
                    let value = value.trim();
                    (key.starts_with("SWING_") && !value.contains("${")).then_some(key)
                })
                .collect()
        }

        let compose = include_str!("../../compose.yaml");
        let found: std::collections::BTreeSet<&str> =
            mirror_environment_literal_swing_vars(compose)
                .into_iter()
                .collect();

        for env_name in &found {
            assert!(
                SETTINGS.iter().any(|s| s.env == *env_name),
                "{env_name}: found in compose.yaml's mirror environment block but is not a catalog env var"
            );
        }

        let listed: std::collections::BTreeSet<&str> = ENV_NO_EFFECT_IN_COMPOSE
            .iter()
            .map(|key| env_of(key))
            .collect();

        assert_eq!(
            found, listed,
            "ENV_NO_EFFECT_IN_COMPOSE is out of sync with compose.yaml's mirror service environment block"
        );
    }

    #[test]
    fn generated_toml_example_parses_to_defaults() {
        let generated = render_toml_example();
        let cfg = crate::config::build_config_from_str(&generated, |_| None)
            .expect("generated swing.example.toml must parse");
        let defaults = crate::config::build_config_from_str("", |_| None).unwrap();

        assert_eq!(cfg.policy, defaults.policy);
        assert_eq!(cfg.kubo, defaults.kubo);
        assert_eq!(cfg.publish, defaults.publish);
        assert_eq!(cfg.gateway, defaults.gateway);
        assert_eq!(cfg.dashboard, defaults.dashboard);
        assert_eq!(cfg.ipfs.api, defaults.ipfs.api);
        assert_eq!(cfg.ipfs.mfs_root, defaults.ipfs.mfs_root);

        assert_eq!(
            cfg.nostr.secret_key.is_none(),
            defaults.nostr.secret_key.is_none()
        );
        assert_eq!(cfg.nostr.relays, defaults.nostr.relays);
        assert_eq!(cfg.nostr.mirror_set, defaults.nostr.mirror_set);
        assert_eq!(cfg.nostr.site_event_kind, defaults.nostr.site_event_kind);
        assert_eq!(
            cfg.nostr.replica_event_kind,
            defaults.nostr.replica_event_kind
        );

        assert_eq!(cfg.agent.state_dir, defaults.agent.state_dir);
        assert_eq!(cfg.agent.poll_interval, defaults.agent.poll_interval);
        assert_eq!(cfg.agent.fetch_timeout, defaults.agent.fetch_timeout);
        assert_eq!(
            cfg.agent.fetch_idle_timeout,
            defaults.agent.fetch_idle_timeout
        );
        assert_eq!(cfg.agent.concurrency, defaults.agent.concurrency);
        assert_eq!(cfg.agent.report_ttl, defaults.agent.report_ttl);
    }
}
