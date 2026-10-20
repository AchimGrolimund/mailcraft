// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! To-do items.

use mailcraft_time::{Date, Timestamp};
use serde::{Deserialize, Serialize};

use crate::{Id, Importance};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Task {
    pub id: Id,
    pub title: String,
    pub notes: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<Date>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub due: Option<Date>,
    /// When it was marked complete.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed: Option<Timestamp>,
    pub importance: Importance,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub categories: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reminder: Option<Timestamp>,
    /// The flagged message this to-do stands for, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<Id>,
    /// iCalendar `UID` (VTODO).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub uid: String,
}

impl Task {
    pub fn is_complete(&self) -> bool {
        self.completed.is_some()
    }

    /// Due before `today` and not done.
    pub fn is_overdue(&self, today: Date) -> bool {
        !self.is_complete() && self.due.is_some_and(|d| d < today)
    }

    /// `Today`, `Tomorrow`, `Overdue`, the weekday within a week, else the date; `No date`.
    pub fn due_label(&self, today: Date) -> String {
        let Some(due) = self.due else { return "No date".into() };
        match today.days_until(due) {
            0 => "Today".into(),
            1 => "Tomorrow".into(),
            -1 => "Yesterday".into(),
            n if (2..7).contains(&n) => due.weekday().name().into(),
            _ => due.medium(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn due_labels_and_overdue() {
        let today = Date::new(2026, 10, 14).unwrap();
        let task = |due: Option<Date>| Task { due, ..Default::default() };
        assert_eq!(task(None).due_label(today), "No date");
        assert_eq!(task(Some(today)).due_label(today), "Today");
        assert_eq!(task(Some(today.add_days(1))).due_label(today), "Tomorrow");
        assert_eq!(task(Some(today.add_days(3))).due_label(today), "Saturday");
        assert_eq!(task(Some(today.add_days(30))).due_label(today), "Nov 13, 2026");
        let mut late = task(Some(today.add_days(-2)));
        assert!(late.is_overdue(today));
        late.completed = Some(1);
        assert!(!late.is_overdue(today) && late.is_complete());
    }
}
