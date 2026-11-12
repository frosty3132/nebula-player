//! Указатель настроек для поиска: каждая строка — вкладка, карточка, название
//! (как в интерфейсе, по нему же ищется строка на странице), подсказка и
//! синонимы, по которым её тоже можно найти.
//!
//! Если в настройках появляется новая строка, её стоит добавить и сюда.

use super::state::SettingsTab;

pub struct Entry {
    pub tab: SettingsTab,
    pub card: &'static str,
    /// Текст строки (`.set-title`) или, для целых карточек, их заголовок.
    pub label: &'static str,
    pub hint: &'static str,
    /// Синонимы и английские слова.
    pub keys: &'static str,
    /// true — это карточка целиком, а не отдельная строка.
    pub whole_card: bool,
}

const fn row(tab: SettingsTab, card: &'static str, label: &'static str, hint: &'static str, keys: &'static str) -> Entry {
    Entry { tab, card, label, hint, keys, whole_card: false }
}

const fn card(tab: SettingsTab, card: &'static str, keys: &'static str) -> Entry {
    Entry { tab, card, label: card, hint: "", keys, whole_card: true }
}

use SettingsTab::*;

pub const INDEX: &[Entry] = &[
    // ── звук ──
    row(Sound, "Громкость", "Громкость", "Выше 100% — цифровое усиление", "volume звук усиление"),
    row(Sound, "Громкость", "Максимум громкости", "", "volume boost 200% 400%"),
    row(Sound, "Громкость", "Кривая громкости", "«Естественная» точнее на малой громкости", "linear perceptual"),
    row(Sound, "Громкость", "Шаг громкости", "Колесо мыши и стрелки ↑/↓", "колесо мышь"),
    row(Sound, "Обработка", "Лимитер", "Мягко ограничивает пики", "limiter компрессор хрип"),
    row(Sound, "Обработка", "Плавность паузы и перемотки", "", "fade затухание"),
    row(Sound, "Обработка", "Баланс", "", "balance лево право стерео"),
    row(Sound, "Обработка", "Моно", "", "mono стерео"),
    row(Sound, "Эквалайзер", "Включить эквалайзер", "", "eq equalizer пресеты бас"),
    row(Sound, "Эквалайзер", "Предусиление", "Уменьшите, если сильно подняли полосы", "preamp eq"),
    row(Sound, "Устройство и качество", "Устройство вывода", "", "device наушники колонки динамики output"),
    row(Sound, "Устройство и качество", "Качество SoundCloud", "", "quality aac mp3 битрейт"),
    row(Sound, "Устройство и качество", "Буфер вперёд", "Сколько секунд трека скачивать заранее", "buffer трафик загрузка"),
    // ── воспроизведение ──
    row(Playback, "Переходы и очередь", "Плавный переход между треками", "Как кроссфейд в Spotify", "crossfade кроссфейд переход"),
    row(Playback, "Переходы и очередь", "Похожие треки после очереди", "Радио по мотивам последнего трека", "autoplay радио related"),
    row(Playback, "Переходы и очередь", "Предзагрузка следующего трека", "", "preload"),
    row(Playback, "Переходы и очередь", "Запоминать очередь и позицию", "", "session сессия перезапуск"),
    row(Playback, "Переходы и очередь", "Пропускать 30-секундные превью", "", "preview превью"),
    row(Playback, "YouTube (yt-dlp)", "Заменять превью SoundCloud полной версией", "Превью и недоступные треки играют целиком с YouTube", "youtube ютуб yt-dlp превью полная версия"),
    row(Playback, "YouTube (yt-dlp)", "yt-dlp", "Скачивается сам и обновляется раз в неделю", "youtube обновить yt-dlp"),
    row(Playback, "Управление", "Номера треков в списках", "", "numbers нумерация"),
    row(Playback, "Управление", "Шаг перемотки", "Стрелки ← / →", "seek"),
    row(Playback, "Управление", "«Назад» перезапускает трек после", "Иначе — переход к предыдущему", "previous назад"),
    // ── текст песен ──
    row(Lyrics, "Панель текста", "Показывать тексты", "", "lyrics lrclib слова"),
    row(Lyrics, "Панель текста", "Открывать панель, когда текст найден", "", "auto open автооткрытие"),
    row(Lyrics, "Панель текста", "Сторона панели", "", "слева справа side"),
    row(Lyrics, "Панель текста", "Ширина панели", "", "width"),
    row(Lyrics, "Оформление", "Размер шрифта", "", "font size текст"),
    row(Lyrics, "Оформление", "Выравнивание", "", "align центр"),
    row(Lyrics, "Оформление", "Караоке-заливка строки", "", "karaoke"),
    row(Lyrics, "Оформление", "Размывать неактивные строки", "", "blur"),
    row(Lyrics, "Синхронизация", "Автопрокрутка", "Пауза на 3 секунды, если вы листаете сами", "autoscroll"),
    row(Lyrics, "Синхронизация", "Показывать несинхронный текст", "", "plain fallback"),
    row(Lyrics, "Синхронизация", "Общий сдвиг", "Добавляется к сдвигу конкретного трека", "offset задержка синхронизация"),
    // ── интерфейс ──
    row(Interface, "Окно", "Прозрачность окна", "Сквозь окно виден рабочий стол", "opacity transparency"),
    row(Interface, "Окно", "Эффект окна Windows", "Mica и Acrylic", "mica acrylic blur"),
    row(Interface, "Окно", "Масштаб интерфейса", "Применяется, когда отпускаешь ползунок", "scale zoom размер"),
    row(Interface, "Окно", "Поверх всех окон", "", "always on top"),
    row(Interface, "Окно", "Островок на рабочем столе", "Свёрнутый плеер — плашка вверху экрана поверх всех окон", "dynamic island островок мини плеер рабочий стол поиск громкость"),
    row(Interface, "Окно", "Сворачивать в трей при закрытии", "Крестик прячет Nebula в трей, музыка играет дальше", "tray трей фон закрытие крестик значок"),
    row(Interface, "Окно", "Часы в «Сцене»", "Кольцо, цифры, стрелки, перекидные или шкала дня", "часы clock тема оформление стрелки циферблат"),
    row(Interface, "Окно", "Погода в островке", "Город для погоды рядом с часами", "погода weather город температура"),
    row(Playback, "Скачивание треков", "Папка для треков", "Куда сохранять скачанные треки", "скачать download папка загрузки сохранить"),
    row(Playback, "Скачивание треков", "Спрашивать, куда сохранить", "Окно выбора папки перед скачиванием", "скачать спрашивать по умолчанию"),
    row(Interface, "Анимации", "Анимации", "", "animation"),
    row(Interface, "Анимации", "Скорость анимаций", "Меньше — быстрее", "speed"),
    row(Interface, "Анимации", "Заставка при запуске", "Логотип, затем интерфейс собирается по частям", "intro splash запуск"),
    row(Interface, "Производительность", "Пауза фона, когда окно неактивно", "Звёзды и видео замирают", "фон нагрузка"),
    row(Interface, "Производительность", "Экономный режим", "Меньше размытия, звёзд и эффектов", "low power ноутбук слабый пк"),
    row(Interface, "Эффекты", "Цвет акцента из обложки", "", "accent dynamic цвет"),
    row(Interface, "Эффекты", "Пульсация обложки в такт басу", "", "beat glow бас"),
    row(Interface, "Эффекты", "Эффект «рыбий глаз» при прокрутке", "Треки у краёв списка сжимаются и наклоняются", "fisheye прокрутка"),
    row(Interface, "Эффекты", "Визуализатор на главной", "Спектр как в cava внутри планеты", "visualizer cava спектр"),
    row(Interface, "Discord Rich Presence", "Включить Discord RPC", "Статус «Слушает» в Discord", "discord rpc статус"),
    // ── темы ──
    card(Themes, "Галерея тем", "theme темы"),
    row(Themes, "Галерея тем", "Тёмная тема", "Влияет на системные элементы", "dark light светлая"),
    card(Themes, "Свой CSS", "css стили"),
    card(Themes, "Цвета", "colors акцент палитра"),
    row(Themes, "Форма и стекло", "Скругление панелей", "", "radius углы"),
    row(Themes, "Форма и стекло", "Скругление обложек", "", "radius cover"),
    row(Themes, "Форма и стекло", "Скругление кнопок", "999 — капсулы", "radius"),
    row(Themes, "Форма и стекло", "Толщина рамок", "", "border"),
    row(Themes, "Форма и стекло", "Жидкое стекло (как в iOS)", "Молочные панели со световым краем", "liquid glass ios"),
    row(Themes, "Форма и стекло", "Размытие стекла", "", "blur glass"),
    row(Themes, "Форма и стекло", "Плотность стекла", "", "glass"),
    row(Themes, "Форма и стекло", "Сила свечения", "", "glow"),
    row(Themes, "Форма и стекло", "Шрифт", "", "font шрифт"),
    row(Themes, "Форма и стекло", "Размер текста", "", "font size"),
    row(Themes, "Фон", "Тип фона", "", "background обои картинка видео"),
    row(Themes, "Фон", "Файл или URL", "Для анимации лучше видео (mp4/webm)", "background gif mp4"),
    row(Themes, "Фон", "Размытие фона", "", "background blur"),
    row(Themes, "Фон", "Затемнение", "", "dim"),
    row(Themes, "Фон", "Туманности", "", "nebula"),
    row(Themes, "Звёзды", "Плотность", "", "stars звёзды"),
    row(Themes, "Звёзды", "Падающие звёзды", "", "stars shooting"),
    row(Themes, "Звёзды", "Мерцание", "", "stars twinkle"),
    // ── расположение ──
    row(Layout, "Панели", "Меню разделов", "Сверху — панель-док со значками", "sidebar dock меню"),
    row(Layout, "Панели", "Компактное меню (только иконки)", "", "compact"),
    row(Layout, "Панели", "Плеер", "Снизу, сверху или парящий", "player плеер положение парящий"),
    row(Layout, "Панели", "Вид плеера", "", "player style"),
    row(Layout, "Панели", "Дизайн плеера Material 3", "Волнистая полоса прогресса", "md3 material"),
    row(Layout, "Панели", "Стиль «Сцена»", "Заголовок с часами и плашкой трека", "сцена stage монохром часы заголовок"),
    row(Layout, "Панели", "Высота плеера", "", "player height"),
    row(Layout, "Панели", "Панель текста", "", "lyrics side"),
    row(Layout, "Контент", "Форма обложки в плеере", "", "cover винил vinyl круг"),
    row(Layout, "Контент", "Плотность списков", "", "density"),
    row(Layout, "Контент", "Размер карточек", "", "cards"),
    card(Layout, "Кнопки управления", "controls кнопки порядок"),
    card(Layout, "Дополнительные кнопки плеера", "extras кнопки"),
    card(Layout, "Пункты меню", "nav меню порядок"),
    card(Layout, "Разделы главной", "home главная разделы"),
    // ── элементы ──
    card(Elements, "Каждый элемент — отдельно", "inspector редактор элементов ctrl+e"),
    // ── аккаунт, кэш, прочее ──
    card(Account, "Аккаунт SoundCloud", "вход логин login выйти soundcloud"),
    card(Account, "Spotify", "spotify спотифай привязать перенос импорт плейлисты лайки любимые"),
    row(Account, "Свои ключи API (для разработчиков)", "Client ID", "", "api ключи developer"),
    row(Cache, "Кэш", "Лимит кэша обложек", "", "cache кэш обложки"),
    row(Cache, "Данные", "Портативный режим", "Все данные хранятся в папке data рядом с exe", "portable"),
    card(Cache, "Данные", "папка настроек сброс reset"),
    card(Hotkeys, "Горячие клавиши", "hotkeys клавиши shortcuts"),
    card(About, "Nebula", "about версия о программе"),
];

/// Нормализация для поиска: нижний регистр, ё → е.
pub fn norm(s: &str) -> String {
    s.to_lowercase().replace('ё', "е")
}

/// Найденные строки, лучшие сначала: совпадение в начале названия, затем в
/// названии, затем в подсказке, карточке, вкладке и синонимах. Каждое слово
/// запроса должно где-то найтись.
pub fn search(query: &str) -> Vec<&'static Entry> {
    let q = norm(query);
    let words: Vec<&str> = q.split_whitespace().collect();
    if words.is_empty() {
        return Vec::new();
    }
    let mut hits: Vec<(u32, usize, &'static Entry)> = INDEX
        .iter()
        .enumerate()
        .filter_map(|(i, e)| {
            let label = norm(e.label);
            let rest = norm(&format!("{} {} {} {}", e.hint, e.card, e.tab.label(), e.keys));
            let mut score = 0u32;
            for w in &words {
                if label.starts_with(w) {
                    score += 30;
                } else if label.split(|c: char| !c.is_alphanumeric()).any(|p| p.starts_with(w)) {
                    score += 20;
                } else if label.contains(w) {
                    score += 12;
                } else if rest.contains(w) {
                    score += 4;
                } else {
                    return None;
                }
            }
            Some((score, i, e))
        })
        .collect();
    hits.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    hits.into_iter().map(|(_, _, e)| e).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_by_label_synonym_and_yo() {
        assert_eq!(search("громк")[0].label, "Громкость");
        assert_eq!(search("crossfade")[0].label, "Плавный переход между треками");
        // «е» находит «ё»
        assert!(search("звезды").iter().any(|e| e.card == "Звёзды"));
        assert!(search("несуществующее слово").is_empty());
    }
}
