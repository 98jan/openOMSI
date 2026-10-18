const NOTICE_KEYS: &[&str] = &[
    "connected to {host}'s session ({players} player(s))",
    "In this session: {players}",
    "{name} joined with {vehicle}",
    "{name} joined",
    "{name} is back",
    "{name} left",
    "{name} lost the connection",
    "{name} was sent away by the host",
    "the host turned us away: {reason}",
    "the host sent you away: {reason}",
    "reconnecting ...",
    "only a joined game can reconnect",
    "asked the server for its administration",
    "the host ended the session",
    "the host has gone quiet; trying to reach it again",
];

pub(super) fn translated_chat_line(line: &str, lookup: impl Fn(&str) -> Option<String>) -> String {
    let Some(notice) = line.strip_prefix("* ") else { return line.into() };
    if let Some(players) = notice.strip_prefix("In this session: ") {
        let Some(translation) = lookup("In this session: {players}") else { return line.into() };
        // Only the first entry carries the host marker; names and vehicle identifiers are data.
        let players = if let Some((host, rest)) = players.split_once(" (host") {
            if rest == ")" || rest.starts_with("), ") || rest.starts_with(", ") {
                if let Some((vehicle, tail)) = rest.strip_prefix(", ").and_then(|r| r.split_once(')')) {
                    lookup("{name} (host, {vehicle})")
                        .map(|t| t.replace("{vehicle}", vehicle).replace("{name}", host) + tail)
                } else {
                    rest.strip_prefix(')').and_then(|tail| lookup("{name} (host)").map(|t| t.replace("{name}", host) + tail))
                }.unwrap_or_else(|| players.into())
            } else {
                players.into()
            }
        } else {
            players.into()
        };
        return format!("* {}", translation.replace("{players}", &players));
    }
    static TEMPLATES: std::sync::OnceLock<Vec<omsi_ui::i18n::Template>> = std::sync::OnceLock::new();
    let templates = TEMPLATES.get_or_init(|| {
        // These are known system notices, so leading placeholders are safe here.
        let mut templates = Vec::new();
        for key in NOTICE_KEYS {
            let mut pieces = vec![omsi_ui::i18n::Piece::Text("* ".into())];
            let mut rest = *key;
            while let Some(open) = rest.find('{') {
                if open > 0 { pieces.push(omsi_ui::i18n::Piece::Text(rest[..open].into())); }
                let close = rest.find('}').unwrap();
                pieces.push(omsi_ui::i18n::Piece::Hole(rest[open + 1..close].into()));
                rest = &rest[close + 1..];
            }
            if !rest.is_empty() { pieces.push(omsi_ui::i18n::Piece::Text(rest.into())); }
            templates.push((key.to_string(), pieces));
        }
        templates.sort_by_key(|(_, pieces)| std::cmp::Reverse(pieces.iter().map(|piece| match piece { omsi_ui::i18n::Piece::Text(text) => text.len(), _ => 0 }).sum::<usize>()));
        templates
    });
    let translated = lookup(notice).or_else(|| omsi_ui::i18n::templated(templates, line, |key| NOTICE_KEYS.contains(&key).then(|| lookup(key)).flatten()));
    translated.map(|text| format!("* {text}")).unwrap_or_else(|| line.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_chat_notices_translate_before_wrapping_and_keep_player_content() {
        let translate = |line| translated_chat_line(line, |key| crate::_rust_i18n_try_translate("zh-tw", key).map(|text| text.into_owned()));
        assert_eq!(translate("* connected to OMSI-Fan's session (2 player(s))"), "* 已連線至 OMSI-Fan 的遊戲（2 位玩家）");
        assert_eq!(translate("* In this session: OMSI-Fan (host, MAN_EN92_main)"), "* 本次連線的玩家：OMSI-Fan（房主，MAN_EN92_main）");
        assert_eq!(translate("* In this session: OMSI-Fan (host), Alice (MAN_NL202)"), "* 本次連線的玩家：OMSI-Fan（房主）, Alice (MAN_NL202)");
        assert_eq!(translate("* Alice {name} joined with MAN_EN92_main"), "* Alice {name} 已加入，駕駛 MAN_EN92_main");
        assert_eq!(translate("* Alice left"), "* Alice 已離開");
        assert_eq!(translate("* reconnecting ..."), "* 正在重新連線...");
        for text in ["Alice: connected to OMSI-Fan's session (2 player(s))", "Alice: In this session: OMSI-Fan (host)", "* A custom server announcement"] {
            assert_eq!(translate(text), text);
        }
        let original = "* connected to OMSI-Fan's session (2 player(s))";
        assert_eq!(translated_chat_line(original, |_| None), original);
    }
}
