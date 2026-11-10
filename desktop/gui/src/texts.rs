//! The Messages page: a phone's text conversations and one conversation's texts, read from the
//! daemon (which asks the phone) off the UI thread, and texts sent from the reply box.

use chrono::{DateTime, Datelike, Local, TimeZone};
use slint::{ComponentHandle, Model, ModelRc, VecModel};

use crate::daemon::{Daemon, DbusDaemon, Inbox, ThreadData};
use crate::{Bubble, Chat, MainWindow, breakable, notify};

/// The page's id in `selected`.
pub const MESSAGES: &str = "messages";
/// While the phone hasn't answered yet, how soon to look again.
const LOOK_AGAIN: std::time::Duration = std::time::Duration::from_secs(2);
/// Snapshots show made-up texts: never read the real ones.
pub static MADE_UP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// Texts closer than this, from the same side, are one run: one time under the last.
const RUN_GAP_MS: i64 = 10 * 60 * 1000;

pub fn wire(ui: &MainWindow) {
    ui.on_show_messages({
        let ui = ui.as_weak();
        move |id| {
            let Some(win) = ui.upgrade() else { return };
            if win.get_current().id != id || win.get_selected() != MESSAGES {
                win.set_chats(ModelRc::default());
                win.set_texts_state("loading".into());
            }
            win.set_thread("".into());
            win.set_draft("".into());
            win.set_texts_phone(win.get_current().name);
            win.set_texts_linked(win.get_current().linked);
            win.set_selected(MESSAGES.into());
            read(&ui);
        }
    });
    ui.on_open_thread({
        let ui = ui.as_weak();
        move |thread| {
            let Some(win) = ui.upgrade() else { return };
            let chats = win.get_chats();
            let name = (0..chats.row_count()).filter_map(|i| chats.row_data(i)).find(|c| c.thread == thread).map(|c| c.name).unwrap_or_default();
            win.set_thread_name(name);
            win.set_thread_number("".into());
            win.set_bubbles(ModelRc::default());
            win.set_draft("".into());
            win.set_thread(thread);
            read(&ui);
        }
    });
    ui.on_send_text({
        let ui = ui.as_weak();
        move |thread, text| {
            let Some(win) = ui.upgrade() else { return };
            let (id, text) = (win.get_current().id.to_string(), text.trim().to_owned());
            let Ok(thread) = thread.parse::<i64>() else { return };
            if text.is_empty() {
                return;
            }
            // Cleared at once (the text shows as Sending… until the phone has it); put back if it can't go.
            win.set_draft("".into());
            let ui = ui.clone();
            std::thread::spawn(move || {
                if let Err(e) = DbusDaemon::connect().and_then(|d| d.send_text(&id, thread, &text)) {
                    notify(&ui, format!("Couldn't send: {e:#}"));
                    let ui = ui.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(win) = ui.upgrade()
                            && win.get_draft().is_empty()
                        {
                            win.set_draft(text.into());
                        }
                    });
                }
            });
        }
    });
}

/// Rereads what the page shows, if it's showing: the conversations, and the open one's texts.
pub fn read(ui: &slint::Weak<MainWindow>) {
    if MADE_UP.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    let ui = ui.clone();
    let _ = slint::invoke_from_event_loop(move || {
        let Some(win) = ui.upgrade() else { return };
        if win.get_selected() != MESSAGES {
            return;
        }
        let (id, thread) = (win.get_current().id.to_string(), win.get_thread().parse::<i64>().ok());
        let linked = win.get_current().linked;
        std::thread::spawn(move || {
            let d = DbusDaemon::connect();
            let inbox = d.as_ref().map_err(|e| anyhow::anyhow!("{e:#}")).and_then(|d| d.conversations(&id));
            let texts = thread.and_then(|t| d.as_ref().ok().and_then(|d| d.thread(&id, t).ok()));
            let _ = slint::invoke_from_event_loop(move || {
                let Some(win) = ui.upgrade() else { return };
                if win.get_selected() != MESSAGES || win.get_current().id != id.as_str() {
                    return;
                }
                let now = Local::now();
                match inbox {
                    Ok(inbox) => show_inbox(&win, &inbox, linked, now),
                    Err(_) => show_inbox(&win, &Inbox { state: if linked { "silent" } else { "away" }.into(), list: Vec::new() }, linked, now),
                }
                if let Some(t) = texts.filter(|_| win.get_thread().parse::<i64>().ok() == thread) {
                    show_thread(&win, &t, now);
                }
                if win.get_texts_state() == "loading" {
                    let ui = win.as_weak();
                    slint::Timer::single_shot(LOOK_AGAIN, move || read(&ui));
                }
            });
        });
    });
}

pub fn show_inbox(ui: &MainWindow, inbox: &Inbox, linked: bool, now: DateTime<Local>) {
    let state = if inbox.list.is_empty() && !linked { "away" } else { inbox.state.as_str() };
    ui.set_texts_state(state.into());
    let chats: Vec<Chat> = inbox
        .list
        .iter()
        .map(|c| Chat {
            thread: c.thread.to_string().into(),
            name: breakable(&c.name).into(),
            initial: initial(&c.name).into(),
            hue: hue(&c.name),
            snippet: if c.mine { format!("You: {}", one_line(&c.snippet)) } else { one_line(&c.snippet) }.into(),
            meta: when(c.date, now).into(),
            unread: c.unread,
        })
        .collect();
    ui.set_chats(ModelRc::new(VecModel::from(chats)));
}

pub fn show_thread(ui: &MainWindow, t: &ThreadData, now: DateTime<Local>) {
    if !t.name.is_empty() {
        ui.set_thread_name(breakable(&t.name).into());
    }
    ui.set_thread_number(t.number.clone().into());
    let today = now.date_naive();
    let day_of = |ms: i64| Local.timestamp_millis_opt(ms).single().map(|d| d.date_naive());
    let mut out: Vec<Bubble> = Vec::with_capacity(t.texts.len());
    for (i, m) in t.texts.iter().enumerate() {
        let day = day_of(m.date);
        let starts_day = i == 0 || day_of(t.texts[i - 1].date) != day;
        let next = t.texts.get(i + 1);
        let ends_run = next.is_none_or(|n| n.mine != m.mine || n.date - m.date > RUN_GAP_MS || day_of(n.date) != day || n.sending != m.sending);
        let meta = if m.sending {
            if next.is_none_or(|n| !n.sending) { "Sending…".to_owned() } else { String::new() }
        } else if m.failed {
            "Not sent".to_owned()
        } else if ends_run {
            Local.timestamp_millis_opt(m.date).single().map(|d| d.format("%H:%M").to_string()).unwrap_or_default()
        } else {
            String::new()
        };
        out.push(Bubble {
            text: breakable(&m.body).into(),
            mine: m.mine,
            day: if starts_day { day.map(|d| day_label(d, today)).unwrap_or_default() } else { String::new() }.into(),
            meta: meta.into(),
            failed: m.failed,
        });
    }
    ui.set_bubbles(ModelRc::new(VecModel::from(out)));
}

/// "Today", "Yesterday", "Monday", "3 November", "3 November 2025".
fn day_label(d: chrono::NaiveDate, today: chrono::NaiveDate) -> String {
    match (today - d).num_days() {
        0 => "Today".to_owned(),
        1 => "Yesterday".to_owned(),
        2..=6 => d.format("%A").to_string(),
        _ if d.year() == today.year() => d.format("%-d %B").to_string(),
        _ => d.format("%-d %B %Y").to_string(),
    }
}

/// A conversation's time in the list: "14:32" today, "Yesterday", "Mon", "3 Nov", "3 Nov 2025".
fn when(ms: i64, now: DateTime<Local>) -> String {
    let Some(at) = Local.timestamp_millis_opt(ms).single() else { return String::new() };
    let (d, today) = (at.date_naive(), now.date_naive());
    match (today - d).num_days() {
        0 => at.format("%H:%M").to_string(),
        1 => "Yesterday".to_owned(),
        2..=6 => at.format("%a").to_string(),
        _ if d.year() == today.year() => at.format("%-d %b").to_string(),
        _ => at.format("%-d %b %Y").to_string(),
    }
}

/// The avatar's letter: the name's first letter, none for a number.
fn initial(name: &str) -> String {
    name.chars().find(|c| !c.is_whitespace()).filter(|c| c.is_alphabetic()).map(|c| c.to_uppercase().collect()).unwrap_or_default()
}

/// The avatar's colour, the same for a name every time.
fn hue(name: &str) -> i32 {
    (name.bytes().fold(7u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32)) % 6) as i32
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_times() {
        assert_eq!(initial(" sam"), "S");
        assert_eq!(initial("+1 555"), "");
        assert_eq!(hue("Sam"), hue("Sam"));
        assert!((0..6).contains(&hue("anyone")));
        assert_eq!(one_line("a\n b  c"), "a b c");
        let now = Local.with_ymd_and_hms(2026, 11, 10, 15, 0, 0).unwrap();
        let ms = |d: u32, h: u32| Local.with_ymd_and_hms(2026, 11, d, h, 5, 0).unwrap().timestamp_millis();
        assert_eq!(when(ms(10, 9), now), "09:05");
        assert_eq!(when(ms(9, 9), now), "Yesterday");
        assert_eq!(when(ms(1, 9), now), "1 Nov");
    }
}
