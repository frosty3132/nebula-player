//! Слияние плейлиста Nebula с плейлистом SoundCloud. Трёхстороннее, как в git:
//! есть снимок на момент прошлой синхронизации (`base`), текущий список в
//! Nebula и текущий список на SoundCloud. Что поменяли только с одной
//! стороны — переносится на другую; если меняли обе, сохраняются и
//! удаления, и добавления с обеих сторон (порядок — как на SoundCloud,
//! новые треки из Nebula встают после своих соседей).

use std::collections::HashSet;

#[derive(Clone, Debug, PartialEq)]
pub struct Merged {
    /// Итоговый порядок треков (urn SoundCloud).
    pub ids: Vec<String>,
    pub title: String,
    /// Отправить на SoundCloud новые треки / новое название.
    pub push_tracks: bool,
    pub push_title: bool,
}

pub fn merge(base: &[String], base_title: &str, local: &[String], local_title: &str, remote: &[String], remote_title: &str) -> Merged {
    let ids = merge_ids(base, local, remote);
    let local_title = local_title.trim();
    let title = if local_title != base_title.trim() && !local_title.is_empty() { local_title.to_string() } else { remote_title.trim().to_string() };
    Merged { push_tracks: ids != remote, push_title: title != remote_title.trim(), ids, title }
}

fn dedup(v: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    v.iter().filter(|x| seen.insert(x.as_str())).cloned().collect()
}

fn merge_ids(base: &[String], local: &[String], remote: &[String]) -> Vec<String> {
    let (local, remote) = (dedup(local), dedup(remote));
    if local == base {
        return remote;
    }
    if remote == base || remote == local {
        return local;
    }
    let base_set: HashSet<&str> = base.iter().map(String::as_str).collect();
    let local_set: HashSet<&str> = local.iter().map(String::as_str).collect();
    // удалённые в Nebula убираем и из списка SoundCloud
    let mut out: Vec<String> = remote.iter().filter(|id| !(base_set.contains(id.as_str()) && !local_set.contains(id.as_str()))).cloned().collect();
    // добавленные в Nebula ставим после ближайшего предыдущего соседа
    for (k, id) in local.iter().enumerate() {
        if base_set.contains(id.as_str()) || out.contains(id) {
            continue;
        }
        let pos = local[..k].iter().rev().find_map(|prev| out.iter().position(|x| x == prev)).map(|p| p + 1).unwrap_or(0);
        out.insert(pos, id.clone());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Vec<String> {
        s.split_whitespace().map(str::to_string).collect()
    }

    #[test]
    fn one_side_changes_win() {
        let m = merge(&v("a b c"), "T", &v("a b c d"), "T", &v("a b c"), "T");
        assert_eq!(m.ids, v("a b c d"));
        assert!(m.push_tracks && !m.push_title);
        let m = merge(&v("a b c"), "T", &v("a b c"), "T", &v("c a"), "T");
        assert_eq!(m.ids, v("c a"));
        assert!(!m.push_tracks);
    }

    #[test]
    fn both_sides_changed() {
        // в Nebula удалили b и добавили x после c; на SoundCloud добавили y в начало
        let m = merge(&v("a b c"), "T", &v("a c x"), "T", &v("y a b c"), "T");
        assert_eq!(m.ids, v("y a c x"));
        assert!(m.push_tracks);
        // добавление в самое начало
        let m = merge(&v("a b"), "T", &v("x a b"), "T", &v("a b z"), "T");
        assert_eq!(m.ids, v("x a b z"));
    }

    #[test]
    fn first_link_takes_union() {
        // при первой связи снимка нет: ничего не теряем, порядок SoundCloud
        let m = merge(&[], "", &v("a x"), "Мой", &v("a b"), "SC");
        assert_eq!(m.ids, v("a x b"));
        assert_eq!(m.title, "Мой");
    }

    #[test]
    fn titles() {
        assert_eq!(merge(&[], "Old", &[], "Old", &[], "New").title, "New");
        let m = merge(&[], "Old", &[], "Mine", &[], "New");
        assert_eq!(m.title, "Mine");
        assert!(m.push_title);
        assert!(!merge(&[], "A", &[], "A", &[], "A").push_title);
    }

    #[test]
    fn duplicates_dropped() {
        let m = merge(&v("a"), "T", &v("a b b"), "T", &v("a"), "T");
        assert_eq!(m.ids, v("a b"));
    }
}
