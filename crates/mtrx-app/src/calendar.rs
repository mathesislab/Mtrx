//! A month of days to pick one from: the send-later picker's (days to
//! come) and the export dialog's (days gone).

use chrono::{Datelike, Months, NaiveDate};
use iced::{
    Alignment, Element, Length,
    widget::{button, column, row, space, text},
};

use crate::{fonts, style};

const DAY_CELL: f32 = 32.0;

/// The first day of `date`'s month.
pub fn first_of_month(date: NaiveDate) -> NaiveDate {
    date.with_day(1).unwrap_or(date)
}

/// The month `by` months from `month` (a first day).
pub fn turn(month: NaiveDate, by: i32) -> Option<NaiveDate> {
    if by < 0 {
        month.checked_sub_months(Months::new(by.unsigned_abs()))
    } else {
        month.checked_add_months(Months::new(by.unsigned_abs()))
    }
}

/// `month` (its first day), Monday first: `selected` marked, `today` in
/// bold, the days `pickable` enabled. The arrows turn the month by what
/// `turn` gives, if anything.
pub fn view<'a, M: Clone + 'a>(
    month: NaiveDate,
    selected: NaiveDate,
    today: NaiveDate,
    pickable: impl Fn(NaiveDate) -> bool,
    turn: impl Fn(i32) -> Option<M>,
    pick: impl Fn(NaiveDate) -> M,
) -> Element<'a, M> {
    let nav = |label: &'static str, by: i32| {
        button(text(label).size(14))
            .padding([0, 8])
            .style(style::text_button)
            .on_press_maybe(turn(by))
    };
    let header = row![
        nav("‹", -1),
        text(month.format("%B %Y").to_string())
            .size(13)
            .font(fonts::SEMIBOLD)
            .width(Length::Fill)
            .center(),
        nav("›", 1),
    ]
    .align_y(Alignment::Center);
    let weekdays = row(["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"].map(|day| {
        text(day)
            .size(11)
            .style(text::secondary)
            .width(DAY_CELL)
            .center()
            .into()
    }));
    let mut weeks = column![header, weekdays].spacing(2).width(DAY_CELL * 7.0);
    let lead = month.weekday().num_days_from_monday();
    let mut day = month - chrono::Days::new(lead.into());
    while day.month() == month.month() || day < month {
        let mut week = row![];
        for _ in 0..7 {
            let cell: Element<'a, M> = if day.month() == month.month() {
                let label = text(day.day().to_string()).size(12).center();
                let label = if day == today {
                    label.font(fonts::SEMIBOLD)
                } else {
                    label
                };
                let is_selected = day == selected;
                button(label.width(Length::Fill))
                    .width(DAY_CELL)
                    .padding([4, 0])
                    .style(move |theme, status| style::wheel_item(theme, status, is_selected))
                    .on_press_maybe(pickable(day).then(|| pick(day)))
                    .into()
            } else {
                space().width(DAY_CELL).into()
            };
            week = week.push(cell);
            day = day.succ_opt().unwrap_or(day);
        }
        weeks = weeks.push(week);
    }
    weeks.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn months_turn_both_ways() {
        let march = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
        assert_eq!(turn(march, -3), NaiveDate::from_ymd_opt(2025, 12, 1));
        assert_eq!(turn(march, 10), NaiveDate::from_ymd_opt(2027, 1, 1));
        assert_eq!(
            first_of_month(NaiveDate::from_ymd_opt(2026, 3, 17).unwrap()),
            march
        );
    }
}
