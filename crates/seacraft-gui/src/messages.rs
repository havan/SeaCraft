//! The message log: errors and warnings from file actions and jobs, typed
//! (level, text, time) instead of status strings that callers would have to
//! match on. The status bar shows the latest line; View > Messages (or a
//! click on the status text) lists them all.

use std::collections::VecDeque;
use std::time::SystemTime;

use eframe::egui;

use crate::theme;

/// How serious a message is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    /// A completed action worth keeping (a file opened, saved or exported).
    Info,
    /// Something went through, but not all of it (a project file left alone).
    Warning,
    /// The action failed.
    Error,
}

impl Level {
    pub fn label(self) -> &'static str {
        match self {
            Level::Info => "info",
            Level::Warning => "warning",
            Level::Error => "error",
        }
    }

    /// The colour the status bar and the log draw this level's text in.
    pub fn colour(self) -> egui::Color32 {
        match self {
            Level::Info => theme::TEXT_PRIMARY,
            Level::Warning => theme::WARN,
            Level::Error => theme::FAIL,
        }
    }
}

/// One logged message.
#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    pub level: Level,
    pub text: String,
    pub time: SystemTime,
}

impl Message {
    /// `HH:MM:SS` (UTC) of [`Message::time`].
    pub fn clock(&self) -> String {
        clock(self.time)
    }
}

/// `HH:MM:SS` in UTC: no time-zone database without another crate, and the
/// log only needs the order and spacing of events.
pub fn clock(time: SystemTime) -> String {
    let secs = time
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
        % 86_400;
    format!(
        "{:02}:{:02}:{:02}",
        secs / 3600,
        (secs / 60) % 60,
        secs % 60
    )
}

/// The newest [`MessageLog::CAP`] messages, oldest first.
#[derive(Debug, Clone, Default)]
pub struct MessageLog {
    entries: VecDeque<Message>,
}

impl MessageLog {
    /// Messages kept; older ones are dropped.
    pub const CAP: usize = 200;

    pub fn push(&mut self, level: Level, text: impl Into<String>) {
        if self.entries.len() == Self::CAP {
            self.entries.pop_front();
        }
        self.entries.push_back(Message {
            level,
            text: text.into(),
            time: SystemTime::now(),
        });
    }

    /// Oldest first.
    pub fn entries(&self) -> impl DoubleEndedIterator<Item = &Message> + ExactSizeIterator {
        self.entries.iter()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn last(&self) -> Option<&Message> {
        self.entries.back()
    }

    /// How many messages are at `level`.
    pub fn count(&self, level: Level) -> usize {
        self.entries.iter().filter(|m| m.level == level).count()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

/// What the Messages window asked for; `App::draw` applies it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessagesAction {
    None,
    Clear,
    Close,
}

/// The Messages window: every logged message, newest first, with Clear.
pub fn messages_window(ctx: &egui::Context, log: &MessageLog) -> MessagesAction {
    let mut action = MessagesAction::None;
    let mut open = true;
    egui::Window::new("Messages")
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_width(560.0)
        .default_height(260.0)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(format!(
                        "{} messages · {} errors · {} warnings (times UTC)",
                        log.len(),
                        log.count(Level::Error),
                        log.count(Level::Warning)
                    ))
                    .color(theme::TEXT_SECONDARY),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add_enabled(!log.is_empty(), egui::Button::new("Clear"))
                        .clicked()
                    {
                        action = MessagesAction::Clear;
                    }
                });
            });
            ui.separator();
            if log.is_empty() {
                ui.label(egui::RichText::new("No messages.").color(theme::TEXT_MUTED));
                return;
            }
            egui::ScrollArea::vertical()
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    egui::Grid::new("messages_grid")
                        .num_columns(3)
                        .striped(true)
                        .show(ui, |ui| {
                            for m in log.entries().rev() {
                                ui.monospace(m.clock());
                                ui.colored_label(m.level.colour(), m.level.label());
                                ui.add(egui::Label::new(&m.text).wrap());
                                ui.end_row();
                            }
                        });
                });
        });
    if !open {
        action = MessagesAction::Close;
    }
    action
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_log_keeps_order_counts_levels_and_drops_the_oldest() {
        let mut log = MessageLog::default();
        assert!(log.is_empty());
        log.push(Level::Info, "saved a.fef");
        log.push(Level::Error, "cannot read b.fef");
        log.push(Level::Warning, "project file not loaded");
        assert_eq!(log.len(), 3);
        assert_eq!(log.count(Level::Error), 1);
        assert_eq!(log.last().unwrap().level, Level::Warning);
        let texts: Vec<&str> = log.entries().map(|m| m.text.as_str()).collect();
        assert_eq!(
            texts,
            [
                "saved a.fef",
                "cannot read b.fef",
                "project file not loaded"
            ]
        );
        for i in 0..MessageLog::CAP {
            log.push(Level::Info, format!("m{i}"));
        }
        assert_eq!(log.len(), MessageLog::CAP);
        assert_eq!(log.entries().next().unwrap().text, "m0");
        log.clear();
        assert!(log.is_empty());
    }

    #[test]
    fn the_clock_is_hours_minutes_seconds() {
        let t = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(86_400 * 3 + 3_723);
        assert_eq!(clock(t), "01:02:03");
    }
}
