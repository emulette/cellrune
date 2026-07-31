use std::sync::Arc;

use crate::CellRange;

macro_rules! table_token_enum {
    (
        $(#[$meta:meta])*
        pub enum $name:ident {
            $($(#[$variant_meta:meta])* $variant:ident => $token:literal),+ $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        #[non_exhaustive]
        pub enum $name {
            $($(#[$variant_meta])* $variant),+
        }

        impl $name {
            pub(crate) fn from_xlsx(value: &str) -> Option<Self> {
                match value {
                    $($token => Some(Self::$variant),)+
                    _ => None,
                }
            }

            /// Returns the corresponding OOXML token.
            pub const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $token,)+
                }
            }
        }
    };
}

table_token_enum! {
    /// Calendar systems accepted by OOXML table value filters.
    pub enum TableCalendarType {
        /// No calendar override.
        None => "none",
        /// Gregorian calendar.
        Gregorian => "gregorian",
        /// US English Gregorian calendar.
        GregorianUs => "gregorianUs",
        /// Middle East French Gregorian calendar.
        GregorianMiddleEastFrench => "gregorianMeFrench",
        /// Arabic Gregorian calendar.
        GregorianArabic => "gregorianArabic",
        /// Hijri calendar.
        Hijri => "hijri",
        /// Hebrew calendar.
        Hebrew => "hebrew",
        /// Taiwan calendar.
        Taiwan => "taiwan",
        /// Japanese emperor-era calendar.
        Japan => "japan",
        /// Thai calendar.
        Thai => "thai",
        /// Korean calendar.
        Korea => "korea",
        /// Saka calendar.
        Saka => "saka",
        /// English transliterated Gregorian calendar.
        GregorianTransliteratedEnglish => "gregorianXlitEnglish",
        /// French transliterated Gregorian calendar.
        GregorianTransliteratedFrench => "gregorianXlitFrench",
    }
}

impl TableIconSet {
    pub(crate) const fn icon_count(self) -> u32 {
        match self {
            Self::ThreeArrows
            | Self::ThreeArrowsGray
            | Self::ThreeFlags
            | Self::ThreeTrafficLights1
            | Self::ThreeTrafficLights2
            | Self::ThreeSigns
            | Self::ThreeSymbols
            | Self::ThreeSymbols2 => 3,
            Self::FourArrows
            | Self::FourArrowsGray
            | Self::FourRedToBlack
            | Self::FourRating
            | Self::FourTrafficLights => 4,
            Self::FiveArrows | Self::FiveArrowsGray | Self::FiveRating | Self::FiveQuarters => 5,
        }
    }
}

table_token_enum! {
    /// Calendar granularity used by one grouped-date filter item.
    pub enum TableDateTimeGrouping {
        /// Group by year.
        Year => "year",
        /// Group by month.
        Month => "month",
        /// Group by day.
        Day => "day",
        /// Group by hour.
        Hour => "hour",
        /// Group by minute.
        Minute => "minute",
        /// Group by second.
        Second => "second",
    }
}

table_token_enum! {
    /// Comparison operators accepted by a custom table filter.
    pub enum TableCustomFilterOperator {
        /// Equality.
        Equal => "equal",
        /// Inequality.
        NotEqual => "notEqual",
        /// Less than.
        LessThan => "lessThan",
        /// Less than or equal.
        LessThanOrEqual => "lessThanOrEqual",
        /// Greater than.
        GreaterThan => "greaterThan",
        /// Greater than or equal.
        GreaterThanOrEqual => "greaterThanOrEqual",
    }
}

table_token_enum! {
    /// Dynamic filter categories defined by OOXML.
    pub enum TableDynamicFilterType {
        /// Values above the average.
        AboveAverage => "aboveAverage",
        /// Values below the average.
        BelowAverage => "belowAverage",
        /// Tomorrow.
        Tomorrow => "tomorrow",
        /// Today.
        Today => "today",
        /// Yesterday.
        Yesterday => "yesterday",
        /// Next week.
        NextWeek => "nextWeek",
        /// This week.
        ThisWeek => "thisWeek",
        /// Last week.
        LastWeek => "lastWeek",
        /// Next month.
        NextMonth => "nextMonth",
        /// This month.
        ThisMonth => "thisMonth",
        /// Last month.
        LastMonth => "lastMonth",
        /// Next quarter.
        NextQuarter => "nextQuarter",
        /// This quarter.
        ThisQuarter => "thisQuarter",
        /// Last quarter.
        LastQuarter => "lastQuarter",
        /// Next year.
        NextYear => "nextYear",
        /// This year.
        ThisYear => "thisYear",
        /// Last year.
        LastYear => "lastYear",
        /// Year to date.
        YearToDate => "yearToDate",
        /// First quarter.
        Quarter1 => "Q1",
        /// Second quarter.
        Quarter2 => "Q2",
        /// Third quarter.
        Quarter3 => "Q3",
        /// Fourth quarter.
        Quarter4 => "Q4",
        /// January.
        Month1 => "M1",
        /// February.
        Month2 => "M2",
        /// March.
        Month3 => "M3",
        /// April.
        Month4 => "M4",
        /// May.
        Month5 => "M5",
        /// June.
        Month6 => "M6",
        /// July.
        Month7 => "M7",
        /// August.
        Month8 => "M8",
        /// September.
        Month9 => "M9",
        /// October.
        Month10 => "M10",
        /// November.
        Month11 => "M11",
        /// December.
        Month12 => "M12",
        /// A null dynamic-filter token.
        Null => "null",
    }
}

table_token_enum! {
    /// Built-in icon sets accepted by OOXML table filters and sorts.
    pub enum TableIconSet {
        /// Three colored arrows.
        ThreeArrows => "3Arrows",
        /// Three gray arrows.
        ThreeArrowsGray => "3ArrowsGray",
        /// Three flags.
        ThreeFlags => "3Flags",
        /// First three-light traffic set.
        ThreeTrafficLights1 => "3TrafficLights1",
        /// Second three-light traffic set.
        ThreeTrafficLights2 => "3TrafficLights2",
        /// Three signs.
        ThreeSigns => "3Signs",
        /// First three-symbol set.
        ThreeSymbols => "3Symbols",
        /// Second three-symbol set.
        ThreeSymbols2 => "3Symbols2",
        /// Four colored arrows.
        FourArrows => "4Arrows",
        /// Four gray arrows.
        FourArrowsGray => "4ArrowsGray",
        /// Four red-to-black indicators.
        FourRedToBlack => "4RedToBlack",
        /// Four ratings.
        FourRating => "4Rating",
        /// Four traffic lights.
        FourTrafficLights => "4TrafficLights",
        /// Five colored arrows.
        FiveArrows => "5Arrows",
        /// Five gray arrows.
        FiveArrowsGray => "5ArrowsGray",
        /// Five ratings.
        FiveRating => "5Rating",
        /// Five quarters.
        FiveQuarters => "5Quarters",
    }
}

table_token_enum! {
    /// Value source used by an OOXML sort condition.
    pub enum TableSortBy {
        /// Cell value.
        Value => "value",
        /// Cell fill color.
        CellColor => "cellColor",
        /// Font color.
        FontColor => "fontColor",
        /// Conditional-format icon.
        Icon => "icon",
    }
}

table_token_enum! {
    /// Sort collation method declared by OOXML.
    pub enum TableSortMethod {
        /// Sort by stroke count.
        Stroke => "stroke",
        /// Sort by phonetic spelling.
        PinYin => "pinYin",
    }
}

/// A validated OOXML double token with its source spelling retained.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableNumericValue(Arc<str>);

impl TableNumericValue {
    pub(crate) fn from_xlsx(value: String) -> Option<Self> {
        let lexical = value.trim_matches(|character| matches!(character, ' ' | '\t' | '\r' | '\n'));
        is_xsd_double(lexical).then(|| Self(Arc::from(value)))
    }

    /// Returns the validated source spelling.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn is_xsd_double(value: &str) -> bool {
    if matches!(value, "INF" | "-INF" | "NaN") {
        return true;
    }
    let bytes = value.as_bytes();
    let mut index = usize::from(matches!(bytes.first(), Some(b'+' | b'-')));
    let integer_start = index;
    while bytes.get(index).is_some_and(u8::is_ascii_digit) {
        index += 1;
    }
    let integer_digits = index - integer_start;
    let mut fraction_digits = 0;
    if bytes.get(index) == Some(&b'.') {
        index += 1;
        let fraction_start = index;
        while bytes.get(index).is_some_and(u8::is_ascii_digit) {
            index += 1;
        }
        fraction_digits = index - fraction_start;
    }
    if integer_digits == 0 && fraction_digits == 0 {
        return false;
    }
    if matches!(bytes.get(index), Some(b'e' | b'E')) {
        index += 1;
        if matches!(bytes.get(index), Some(b'+' | b'-')) {
            index += 1;
        }
        let exponent_start = index;
        while bytes.get(index).is_some_and(u8::is_ascii_digit) {
            index += 1;
        }
        if index == exponent_start {
            return false;
        }
    }
    index == bytes.len()
}

/// A validated OOXML `xsd:dateTime` token with its source spelling retained.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableDateTimeValue(Arc<str>);

impl TableDateTimeValue {
    pub(crate) fn from_xlsx(value: String) -> Option<Self> {
        let lexical = value.trim_matches(|character| matches!(character, ' ' | '\t' | '\r' | '\n'));
        is_xsd_date_time(lexical).then(|| Self(Arc::from(value)))
    }

    /// Returns the validated source spelling.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn is_xsd_date_time(value: &str) -> bool {
    let Some((date, time_with_zone)) = value.split_once('T') else {
        return false;
    };
    if date.contains('T') || time_with_zone.contains('T') {
        return false;
    }

    let (time, timezone) = if let Some(time) = time_with_zone.strip_suffix('Z') {
        (time, Some("Z"))
    } else if let Some(index) = time_with_zone.rfind(['+', '-']) {
        let (time, timezone) = time_with_zone.split_at(index);
        (time, Some(timezone))
    } else {
        (time_with_zone, None)
    };
    if timezone.is_some_and(|timezone| !is_xsd_timezone(timezone)) {
        return false;
    }

    let date = date.strip_prefix('-').unwrap_or(date);
    let mut date_parts = date.split('-');
    let (Some(year), Some(month), Some(day), None) = (
        date_parts.next(),
        date_parts.next(),
        date_parts.next(),
        date_parts.next(),
    ) else {
        return false;
    };
    if year.len() < 4
        || !year.bytes().all(|byte| byte.is_ascii_digit())
        || year.bytes().all(|byte| byte == b'0')
        || (year.len() > 4 && year.starts_with('0'))
        || month.len() != 2
        || day.len() != 2
    {
        return false;
    }
    let (Some(month), Some(day)) = (parse_two_digits(month), parse_two_digits(day)) else {
        return false;
    };
    if !(1..=12).contains(&month) || !(1..=days_in_month(year, month)).contains(&day) {
        return false;
    }

    let mut time_parts = time.split(':');
    let (Some(hour), Some(minute), Some(second), None) = (
        time_parts.next(),
        time_parts.next(),
        time_parts.next(),
        time_parts.next(),
    ) else {
        return false;
    };
    if hour.len() != 2 || minute.len() != 2 {
        return false;
    }
    let (Some(hour), Some(minute)) = (parse_two_digits(hour), parse_two_digits(minute)) else {
        return false;
    };
    let (second, fraction) = second
        .split_once('.')
        .map_or((second, None), |(second, fraction)| {
            (second, Some(fraction))
        });
    let Some(second) = parse_two_digits(second) else {
        return false;
    };
    if minute > 59
        || second > 59
        || fraction.is_some_and(|fraction| {
            fraction.is_empty() || !fraction.bytes().all(|byte| byte.is_ascii_digit())
        })
    {
        return false;
    }
    hour < 24
        || (hour == 24
            && minute == 0
            && second == 0
            && fraction.is_none_or(|fraction| fraction.bytes().all(|byte| byte == b'0')))
}

fn is_xsd_timezone(value: &str) -> bool {
    if value == "Z" {
        return true;
    }
    let bytes = value.as_bytes();
    if bytes.len() != 6
        || !matches!(bytes[0], b'+' | b'-')
        || bytes[3] != b':'
        || !bytes[1..3].iter().all(u8::is_ascii_digit)
        || !bytes[4..6].iter().all(u8::is_ascii_digit)
    {
        return false;
    }
    let hours = (bytes[1] - b'0') * 10 + (bytes[2] - b'0');
    let minutes = (bytes[4] - b'0') * 10 + (bytes[5] - b'0');
    minutes <= 59 && (hours < 14 || (hours == 14 && minutes == 0))
}

fn parse_two_digits(value: &str) -> Option<u8> {
    let bytes: [u8; 2] = value.as_bytes().try_into().ok()?;
    bytes
        .iter()
        .all(u8::is_ascii_digit)
        .then(|| (bytes[0] - b'0') * 10 + (bytes[1] - b'0'))
}

fn days_in_month(year: &str, month: u8) -> u8 {
    match month {
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 31,
    }
}

fn is_leap_year(year: &str) -> bool {
    divisible_by(year, 4) && (!divisible_by(year, 100) || divisible_by(year, 400))
}

fn divisible_by(value: &str, divisor: u16) -> bool {
    value.bytes().fold(0_u16, |remainder, byte| {
        (remainder * 10 + u16::from(byte - b'0')) % divisor
    }) == 0
}

/// One value or grouped date selected by a table auto-filter.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum TableFilterItem {
    /// A literal filter value.
    Value(Option<Arc<str>>),
    /// A grouped calendar value.
    DateGroup(TableDateGroupItem),
}

/// One grouped calendar value selected by a table auto-filter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableDateGroupItem {
    year: u16,
    month: Option<u16>,
    day: Option<u16>,
    hour: Option<u16>,
    minute: Option<u16>,
    second: Option<u16>,
    grouping: TableDateTimeGrouping,
}

impl TableDateGroupItem {
    pub(crate) fn from_xlsx(
        year: u16,
        month: Option<u16>,
        day: Option<u16>,
        hour: Option<u16>,
        minute: Option<u16>,
        second: Option<u16>,
        grouping: TableDateTimeGrouping,
    ) -> Option<Self> {
        if month.is_some_and(|value| !(1..=12).contains(&value))
            || day.is_some_and(|value| !(1..=31).contains(&value))
            || hour.is_some_and(|value| value > 23)
            || minute.is_some_and(|value| value > 59)
            || second.is_some_and(|value| value > 59)
        {
            return None;
        }
        let required_fields_present = match grouping {
            TableDateTimeGrouping::Year => true,
            TableDateTimeGrouping::Month => month.is_some(),
            TableDateTimeGrouping::Day => month.is_some() && day.is_some(),
            TableDateTimeGrouping::Hour => month.is_some() && day.is_some() && hour.is_some(),
            TableDateTimeGrouping::Minute => {
                month.is_some() && day.is_some() && hour.is_some() && minute.is_some()
            }
            TableDateTimeGrouping::Second => {
                month.is_some()
                    && day.is_some()
                    && hour.is_some()
                    && minute.is_some()
                    && second.is_some()
            }
        };
        required_fields_present.then_some(Self {
            year,
            month,
            day,
            hour,
            minute,
            second,
            grouping,
        })
    }

    /// Returns the required calendar year.
    pub const fn year(&self) -> u16 {
        self.year
    }

    /// Returns the optional calendar month.
    pub const fn month(&self) -> Option<u16> {
        self.month
    }

    /// Returns the optional day of month.
    pub const fn day(&self) -> Option<u16> {
        self.day
    }

    /// Returns the optional hour.
    pub const fn hour(&self) -> Option<u16> {
        self.hour
    }

    /// Returns the optional minute.
    pub const fn minute(&self) -> Option<u16> {
        self.minute
    }

    /// Returns the optional second.
    pub const fn second(&self) -> Option<u16> {
        self.second
    }

    /// Returns the OOXML date-time grouping token.
    pub const fn grouping(&self) -> TableDateTimeGrouping {
        self.grouping
    }
}

/// Literal and grouped-date selections for one filter column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableValueFilters {
    blank: bool,
    calendar_type: Option<TableCalendarType>,
    items: Vec<TableFilterItem>,
}

impl TableValueFilters {
    pub(crate) fn from_xlsx(
        blank: bool,
        calendar_type: Option<TableCalendarType>,
        items: Vec<TableFilterItem>,
    ) -> Self {
        Self {
            blank,
            calendar_type,
            items,
        }
    }

    /// Returns whether blank cells are selected.
    pub const fn blank(&self) -> bool {
        self.blank
    }

    /// Returns the optional OOXML calendar type token.
    pub const fn calendar_type(&self) -> Option<TableCalendarType> {
        self.calendar_type
    }

    /// Returns filter values in declaration order.
    pub fn items(&self) -> &[TableFilterItem] {
        &self.items
    }

    pub(crate) fn push_item(&mut self, item: TableFilterItem) {
        self.items.push(item);
    }

    fn clone_cancellable(&self, cancelled: &impl Fn() -> bool) -> Result<Self, ()> {
        let mut items = Vec::with_capacity(self.items.len());
        for item in &self.items {
            if cancelled() {
                return Err(());
            }
            items.push(item.clone());
        }
        Ok(Self {
            blank: self.blank,
            calendar_type: self.calendar_type,
            items,
        })
    }
}

/// One comparison used by a custom table filter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableCustomFilter {
    operator: Option<TableCustomFilterOperator>,
    value: Option<Arc<str>>,
}

impl TableCustomFilter {
    pub(crate) fn from_xlsx(
        operator: Option<TableCustomFilterOperator>,
        value: Option<String>,
    ) -> Self {
        Self {
            operator,
            value: value.map(Arc::from),
        }
    }

    /// Returns the optional OOXML comparison-operator token.
    pub const fn operator(&self) -> Option<TableCustomFilterOperator> {
        self.operator
    }

    /// Returns the comparison value.
    pub fn value(&self) -> Option<&str> {
        self.value.as_deref()
    }
}

/// Custom comparisons for one filter column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableCustomFilters {
    and: bool,
    filters: Vec<TableCustomFilter>,
}

impl TableCustomFilters {
    pub(crate) fn from_xlsx(and: bool, filters: Vec<TableCustomFilter>) -> Self {
        Self { and, filters }
    }

    /// Returns whether all comparisons must match.
    pub const fn and(&self) -> bool {
        self.and
    }

    /// Returns comparisons in declaration order.
    pub fn filters(&self) -> &[TableCustomFilter] {
        &self.filters
    }

    pub(crate) fn push_filter(&mut self, filter: TableCustomFilter) {
        self.filters.push(filter);
    }

    fn clone_cancellable(&self, cancelled: &impl Fn() -> bool) -> Result<Self, ()> {
        let mut filters = Vec::with_capacity(self.filters.len());
        for filter in &self.filters {
            if cancelled() {
                return Err(());
            }
            filters.push(filter.clone());
        }
        Ok(Self {
            and: self.and,
            filters,
        })
    }
}

/// A dynamic date or numeric filter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableDynamicFilter {
    kind: TableDynamicFilterType,
    value: Option<TableNumericValue>,
    iso_value: Option<TableDateTimeValue>,
    max_value: Option<TableNumericValue>,
    max_iso_value: Option<TableDateTimeValue>,
}

impl TableDynamicFilter {
    pub(crate) fn from_xlsx(
        kind: TableDynamicFilterType,
        value: Option<TableNumericValue>,
        iso_value: Option<TableDateTimeValue>,
        max_value: Option<TableNumericValue>,
        max_iso_value: Option<TableDateTimeValue>,
    ) -> Self {
        Self {
            kind,
            value,
            iso_value,
            max_value,
            max_iso_value,
        }
    }

    /// Returns the OOXML dynamic-filter type token.
    pub const fn kind(&self) -> TableDynamicFilterType {
        self.kind
    }

    /// Returns the optional lower or single comparison value.
    pub const fn value(&self) -> Option<&TableNumericValue> {
        self.value.as_ref()
    }

    /// Returns the optional ISO lower or single comparison value.
    pub const fn iso_value(&self) -> Option<&TableDateTimeValue> {
        self.iso_value.as_ref()
    }

    /// Returns the optional upper comparison value.
    pub const fn max_value(&self) -> Option<&TableNumericValue> {
        self.max_value.as_ref()
    }

    /// Returns the optional ISO upper comparison value.
    pub const fn max_iso_value(&self) -> Option<&TableDateTimeValue> {
        self.max_iso_value.as_ref()
    }
}

/// A differential-format color filter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableColorFilter {
    differential_format_id: Option<u32>,
    cell_color: bool,
}

impl TableColorFilter {
    pub(crate) const fn from_xlsx(differential_format_id: Option<u32>, cell_color: bool) -> Self {
        Self {
            differential_format_id,
            cell_color,
        }
    }

    /// Returns the differential-format identifier.
    pub const fn differential_format_id(&self) -> Option<u32> {
        self.differential_format_id
    }

    /// Returns whether the filter targets cell fill rather than font color.
    pub const fn cell_color(&self) -> bool {
        self.cell_color
    }
}

/// An icon-set filter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableIconFilter {
    icon_set: TableIconSet,
    icon_id: Option<u32>,
}

impl TableIconFilter {
    pub(crate) const fn from_xlsx(icon_set: TableIconSet, icon_id: Option<u32>) -> Self {
        Self { icon_set, icon_id }
    }

    /// Returns the OOXML icon-set token.
    pub const fn icon_set(&self) -> TableIconSet {
        self.icon_set
    }

    /// Returns the zero-based icon identifier within the set.
    pub const fn icon_id(&self) -> Option<u32> {
        self.icon_id
    }
}

/// A top/bottom count or percentage filter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableTopFilter {
    top: bool,
    percent: bool,
    value: TableNumericValue,
    filter_value: Option<TableNumericValue>,
}

impl TableTopFilter {
    pub(crate) fn from_xlsx(
        top: bool,
        percent: bool,
        value: TableNumericValue,
        filter_value: Option<TableNumericValue>,
    ) -> Self {
        Self {
            top,
            percent,
            value,
            filter_value,
        }
    }

    /// Returns whether the filter selects the highest rather than lowest values.
    pub const fn top(&self) -> bool {
        self.top
    }

    /// Returns whether the threshold is a percentage.
    pub const fn percent(&self) -> bool {
        self.percent
    }

    /// Returns the requested count or percentage token.
    pub const fn value(&self) -> &TableNumericValue {
        &self.value
    }

    /// Returns the producer-computed threshold value, when present.
    pub const fn filter_value(&self) -> Option<&TableNumericValue> {
        self.filter_value.as_ref()
    }
}

/// The typed filtering rule attached to one auto-filter column.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum TableFilterCriteria {
    /// Literal values and grouped dates.
    Values(TableValueFilters),
    /// One or two custom comparisons.
    Custom(TableCustomFilters),
    /// A dynamic date or numeric filter.
    Dynamic(TableDynamicFilter),
    /// A differential-format color filter.
    Color(TableColorFilter),
    /// An icon-set filter.
    Icon(TableIconFilter),
    /// A top/bottom count or percentage filter.
    Top(TableTopFilter),
}

impl TableFilterCriteria {
    fn clone_cancellable(&self, cancelled: &impl Fn() -> bool) -> Result<Self, ()> {
        match self {
            Self::Values(filters) => Ok(Self::Values(filters.clone_cancellable(cancelled)?)),
            Self::Custom(filters) => Ok(Self::Custom(filters.clone_cancellable(cancelled)?)),
            Self::Dynamic(filter) => Ok(Self::Dynamic(filter.clone())),
            Self::Color(filter) => Ok(Self::Color(filter.clone())),
            Self::Icon(filter) => Ok(Self::Icon(filter.clone())),
            Self::Top(filter) => Ok(Self::Top(filter.clone())),
        }
    }
}

/// One zero-based column selector and its typed filtering rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableFilterColumn {
    column_id: u32,
    hidden_button: bool,
    show_button: bool,
    criteria: Option<TableFilterCriteria>,
}

impl TableFilterColumn {
    pub(crate) const fn from_xlsx(
        column_id: u32,
        hidden_button: bool,
        show_button: bool,
        criteria: Option<TableFilterCriteria>,
    ) -> Self {
        Self {
            column_id,
            hidden_button,
            show_button,
            criteria,
        }
    }

    /// Returns the zero-based column identifier.
    pub const fn column_id(&self) -> u32 {
        self.column_id
    }

    /// Returns whether the filter button is hidden.
    pub const fn hidden_button(&self) -> bool {
        self.hidden_button
    }

    /// Returns whether the filter button is shown.
    pub const fn show_button(&self) -> bool {
        self.show_button
    }

    /// Returns the typed filtering rule, when one is declared.
    pub const fn criteria(&self) -> Option<&TableFilterCriteria> {
        self.criteria.as_ref()
    }

    pub(crate) fn set_criteria(&mut self, criteria: TableFilterCriteria) {
        self.criteria = Some(criteria);
    }

    pub(crate) fn criteria_mut(&mut self) -> Option<&mut TableFilterCriteria> {
        self.criteria.as_mut()
    }

    fn clone_cancellable(&self, cancelled: &impl Fn() -> bool) -> Result<Self, ()> {
        Ok(Self {
            column_id: self.column_id,
            hidden_button: self.hidden_button,
            show_button: self.show_button,
            criteria: self
                .criteria
                .as_ref()
                .map(|criteria| criteria.clone_cancellable(cancelled))
                .transpose()?,
        })
    }
}

/// One typed sort condition within a table sort state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableSortCondition {
    range: CellRange,
    descending: bool,
    sort_by: Option<TableSortBy>,
    custom_list: Option<Arc<str>>,
    differential_format_id: Option<u32>,
    icon_set: Option<TableIconSet>,
    icon_id: Option<u32>,
}

impl TableSortCondition {
    pub(crate) fn from_xlsx(
        range: CellRange,
        descending: bool,
        sort_by: Option<TableSortBy>,
        custom_list: Option<String>,
        differential_format_id: Option<u32>,
        icon_set: Option<TableIconSet>,
        icon_id: Option<u32>,
    ) -> Self {
        Self {
            range,
            descending,
            sort_by,
            custom_list: custom_list.map(Arc::from),
            differential_format_id,
            icon_set,
            icon_id,
        }
    }

    /// Returns the cells compared by this condition.
    pub const fn range(&self) -> CellRange {
        self.range
    }

    /// Returns whether the condition sorts descending.
    pub const fn descending(&self) -> bool {
        self.descending
    }

    /// Returns the optional OOXML sort-by token.
    pub const fn sort_by(&self) -> Option<TableSortBy> {
        self.sort_by
    }

    /// Returns the optional producer custom-list text.
    pub fn custom_list(&self) -> Option<&str> {
        self.custom_list.as_deref()
    }

    /// Returns the optional differential-format identifier.
    pub const fn differential_format_id(&self) -> Option<u32> {
        self.differential_format_id
    }

    /// Returns the optional OOXML icon-set token.
    pub const fn icon_set(&self) -> Option<TableIconSet> {
        self.icon_set
    }

    /// Returns the optional icon identifier.
    pub const fn icon_id(&self) -> Option<u32> {
        self.icon_id
    }

    fn resized(&self, old_data_range: CellRange, new_data_range: CellRange) -> Result<Self, ()> {
        let range = if self.range.start().row() == old_data_range.start().row()
            && self.range.end().row() == old_data_range.end().row()
        {
            CellRange::from_ordered(
                crate::CellAddress::new(new_data_range.start().row(), self.range.start().column()),
                crate::CellAddress::new(new_data_range.end().row(), self.range.end().column()),
            )
        } else if self.range.start().column() == old_data_range.start().column()
            && self.range.end().column() == old_data_range.end().column()
        {
            let start_offset = self
                .range
                .start()
                .row()
                .get()
                .checked_sub(old_data_range.start().row().get())
                .ok_or(())?;
            let end_offset = self
                .range
                .end()
                .row()
                .get()
                .checked_sub(old_data_range.start().row().get())
                .ok_or(())?;
            let start = crate::Row::new(
                new_data_range
                    .start()
                    .row()
                    .get()
                    .checked_add(start_offset)
                    .ok_or(())?,
            )
            .map_err(|_| ())?;
            let end = crate::Row::new(
                new_data_range
                    .start()
                    .row()
                    .get()
                    .checked_add(end_offset)
                    .ok_or(())?,
            )
            .map_err(|_| ())?;
            if end > new_data_range.end().row() {
                return Err(());
            }
            CellRange::from_ordered(
                crate::CellAddress::new(start, new_data_range.start().column()),
                crate::CellAddress::new(end, new_data_range.end().column()),
            )
        } else {
            return Err(());
        };
        Ok(Self {
            range,
            descending: self.descending,
            sort_by: self.sort_by,
            custom_list: self.custom_list.clone(),
            differential_format_id: self.differential_format_id,
            icon_set: self.icon_set,
            icon_id: self.icon_id,
        })
    }
}

/// Sort metadata attached to a table or auto-filter definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableSortState {
    range: CellRange,
    case_sensitive: bool,
    column_sort: bool,
    sort_method: Option<TableSortMethod>,
    conditions: Vec<TableSortCondition>,
}

impl TableSortState {
    pub(crate) fn from_xlsx(
        range: CellRange,
        case_sensitive: bool,
        column_sort: bool,
        sort_method: Option<TableSortMethod>,
        conditions: Vec<TableSortCondition>,
    ) -> Self {
        Self {
            range,
            case_sensitive,
            column_sort,
            sort_method,
            conditions,
        }
    }

    /// Returns the range covered by this sort state.
    pub const fn range(&self) -> CellRange {
        self.range
    }

    /// Returns whether comparisons are case-sensitive.
    pub const fn case_sensitive(&self) -> bool {
        self.case_sensitive
    }

    /// Returns whether the producer declared a column-oriented sort.
    pub const fn column_sort(&self) -> bool {
        self.column_sort
    }

    /// Returns the optional OOXML sort-method token.
    pub const fn sort_method(&self) -> Option<TableSortMethod> {
        self.sort_method
    }

    /// Returns sort conditions in declaration order.
    pub fn conditions(&self) -> &[TableSortCondition] {
        &self.conditions
    }

    pub(crate) fn clone_cancellable(&self, cancelled: &impl Fn() -> bool) -> Result<Self, ()> {
        let mut conditions = Vec::with_capacity(self.conditions.len());
        for condition in &self.conditions {
            if cancelled() {
                return Err(());
            }
            conditions.push(condition.clone());
        }
        Ok(Self {
            range: self.range,
            case_sensitive: self.case_sensitive,
            column_sort: self.column_sort,
            sort_method: self.sort_method,
            conditions,
        })
    }

    pub(crate) fn resized(
        &self,
        old_data_range: CellRange,
        new_data_range: CellRange,
    ) -> Result<Self, ()> {
        let conditions = self
            .conditions
            .iter()
            .map(|condition| condition.resized(old_data_range, new_data_range))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            range: new_data_range,
            case_sensitive: self.case_sensitive,
            column_sort: self.column_sort,
            sort_method: self.sort_method,
            conditions,
        })
    }
}

/// Typed auto-filter metadata attached to one table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableAutoFilter {
    range: CellRange,
    range_is_explicit: bool,
    filter_columns: Vec<TableFilterColumn>,
    sort_state: Option<TableSortState>,
}

impl TableAutoFilter {
    pub(crate) fn from_xlsx(
        range: CellRange,
        range_is_explicit: bool,
        filter_columns: Vec<TableFilterColumn>,
        sort_state: Option<TableSortState>,
    ) -> Self {
        Self {
            range,
            range_is_explicit,
            filter_columns,
            sort_state,
        }
    }

    /// Returns the range controlled by the filter.
    pub const fn range(&self) -> CellRange {
        self.range
    }

    /// Returns the declared filter range, or `None` when it was inherited from the table.
    pub const fn declared_range(&self) -> Option<CellRange> {
        if self.range_is_explicit {
            Some(self.range)
        } else {
            None
        }
    }

    /// Returns filter-column definitions in declaration order.
    pub fn filter_columns(&self) -> &[TableFilterColumn] {
        &self.filter_columns
    }

    /// Returns nested filter sort metadata, when present.
    pub const fn sort_state(&self) -> Option<&TableSortState> {
        self.sort_state.as_ref()
    }

    pub(crate) fn clone_cancellable(&self, cancelled: &impl Fn() -> bool) -> Result<Self, ()> {
        let mut filter_columns = Vec::with_capacity(self.filter_columns.len());
        for column in &self.filter_columns {
            if cancelled() {
                return Err(());
            }
            filter_columns.push(column.clone_cancellable(cancelled)?);
        }
        Ok(Self {
            range: self.range,
            range_is_explicit: self.range_is_explicit,
            filter_columns,
            sort_state: self
                .sort_state
                .as_ref()
                .map(|sort| sort.clone_cancellable(cancelled))
                .transpose()?,
        })
    }

    pub(crate) fn resized(
        &self,
        range: CellRange,
        old_data_range: CellRange,
        new_data_range: CellRange,
    ) -> Result<Self, ()> {
        Ok(Self {
            range,
            range_is_explicit: self.range_is_explicit,
            filter_columns: self.filter_columns.clone(),
            sort_state: self
                .sort_state
                .as_ref()
                .map(|sort| sort.resized(old_data_range, new_data_range))
                .transpose()?,
        })
    }

    pub(crate) fn resized_from_empty(&self, range: CellRange) -> Self {
        debug_assert!(self.sort_state.is_none());
        Self {
            range,
            range_is_explicit: self.range_is_explicit,
            filter_columns: self.filter_columns.clone(),
            sort_state: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{TableDateTimeValue, TableNumericValue};

    #[test]
    fn numeric_filter_tokens_follow_the_xsd_double_lexical_space() {
        for value in [
            "0",
            "-0",
            "+.5",
            "5.",
            "1e3",
            "-2.5E-4",
            "INF",
            "-INF",
            "NaN",
            " \t1.5\r\n",
        ] {
            let parsed = TableNumericValue::from_xlsx(value.to_owned()).expect("valid xsd:double");
            assert_eq!(parsed.as_str(), value);
        }
        for value in ["", ".", "+", "inf", "Infinity", "1e", "1_0", "  "] {
            assert!(
                TableNumericValue::from_xlsx(value.to_owned()).is_none(),
                "{value:?}"
            );
        }
    }

    #[test]
    fn date_time_filter_tokens_follow_the_xsd_lexical_space() {
        for value in [
            "2026-07-31T15:30:45",
            "2026-07-31T15:30:45Z",
            "2026-07-31T15:30:45.125+09:00",
            "-0001-12-31T24:00:00-14:00",
            " \t2000-02-29T00:00:00\r\n",
        ] {
            let parsed =
                TableDateTimeValue::from_xlsx(value.to_owned()).expect("valid xsd:dateTime");
            assert_eq!(parsed.as_str(), value);
        }
        for value in [
            "",
            "2026-02-29T00:00:00",
            "0000-01-01T00:00:00",
            "02026-01-01T00:00:00",
            "2026-07-31 15:30:45",
            "2026-07-31T24:00:01",
            "2026-07-31T15:60:00",
            "2026-07-31T15:30:60",
            "2026-07-31T15:30:45+14:01",
            "2026-07-31T15:30:45z",
        ] {
            assert!(
                TableDateTimeValue::from_xlsx(value.to_owned()).is_none(),
                "{value:?}"
            );
        }
    }
}
