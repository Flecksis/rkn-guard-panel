use anyhow::{Result, ensure};
use std::collections::BTreeMap;

pub type Catalog = BTreeMap<(bool, String), bool>;

pub fn rules(output: &str, exception: bool) -> Result<Catalog> {
    let mut rows = Catalog::new();
    if output.trim()
        == if exception {
            "Исключений нет."
        } else {
            "Ручных банов нет."
        }
    {
        return Ok(rows);
    }
    for line in output.lines().filter(|s| !s.trim().is_empty()) {
        let mut parts = line.split_whitespace();
        let ip = parts.next().unwrap().parse::<std::net::IpAddr>()?;
        let mut value = ip.to_string();
        if exception {
            let protocol = parts.next().unwrap_or("").to_ascii_lowercase();
            let port = parts.next().unwrap_or("").parse::<u16>()?;
            ensure!(
                matches!(protocol.as_str(), "tcp" | "udp") && port > 0,
                "Некорректное исключение"
            );
            value = format!("{ip} {protocol} {port}");
        }
        ensure!(parts.next().is_none(), "Некорректное правило");
        rows.insert((exception, value), true);
    }
    ensure!(!rows.is_empty(), "Сервер не вернул список правил");
    Ok(rows)
}

// Форматы команд list и base-list уже доступны на серверах с управлением из панели.
pub fn parse(output: &str, base: bool) -> Result<Catalog> {
    let mut rows = Catalog::new();
    let mut lines = output.lines();
    let mut empty = false;
    let mut expected = None;
    while let Some(line) = lines.next() {
        let line = line.trim();
        ensure!(
            line != "Показан конец вывода.",
            "Сервер вернул неполный список"
        );
        if let Some(summary) = line.strip_prefix("Всего: ") {
            expected = Some(
                summary
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .parse::<usize>()?,
            );
        }
        empty |= line == "Базовые списки не заданы."
            || line == "Дополнительные списки пока не добавлены.";
        let Some((number, status)) = line.split_once(". ● ") else {
            continue;
        };
        number.parse::<usize>()?;
        let enabled = match status.to_lowercase().as_str() {
            "включён" => true,
            "выключен" | "отключён" => false,
            _ => anyhow::bail!("Неизвестное состояние списка"),
        };
        let url = lines.next().unwrap_or("").trim();
        ensure!(
            url.starts_with("https://") || url.starts_with("http://"),
            "Нет адреса источника"
        );
        ensure!(
            rows.insert((base, url.into()), enabled).is_none(),
            "Источник повторяется"
        );
    }
    ensure!(
        !rows.is_empty() || empty,
        "Сервер не вернул каталог списков"
    );
    ensure!(
        expected.is_none_or(|n| n == rows.len()),
        "Сервер вернул неполный список"
    );
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ручные_баны_и_исключения() {
        assert!(
            rules("2001:db8::1", false)
                .unwrap()
                .contains_key(&(false, "2001:db8::1".into()))
        );
        assert_eq!(
            rules("192.0.2.1 tcp 80\n192.0.2.1 udp 80", true)
                .unwrap()
                .len(),
            2
        );
        assert!(rules("Исключений нет.", true).unwrap().is_empty());
        assert!(rules("Показан конец вывода.\n192.0.2.1", false).is_err());
    }
    #[test]
    fn состояния_списков() {
        let url = "https://example.org/list";
        let base = parse(&format!("1. ● Отключён\n   {url}\n"), true).unwrap();
        let extra = parse(
            &format!("Всего: 1 · Включено: 1\n\n7. ● включён\n   {url}\n   Подсетей в копии: 2\n"),
            false,
        )
        .unwrap();
        assert_eq!(base[&(true, url.into())], false);
        assert!(extra[&(false, url.into())]);
        assert!(
            parse(
                "Всего: 0 · Включено: 0\nДополнительные списки пока не добавлены.",
                false
            )
            .unwrap()
            .is_empty()
        );
        assert!(
            parse(
                "Показан конец вывода.\n1. ● включён\nhttps://example.org",
                false
            )
            .is_err()
        );
    }
}
