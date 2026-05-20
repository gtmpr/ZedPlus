use crate::config::schema::LocaleConfig;
use chrono::{DateTime, Local, Offset};
use chrono_tz::Tz;

pub struct LocaleContext {
    pub config: LocaleConfig,
}

impl LocaleContext {
    pub fn new(config: LocaleConfig) -> Self {
        Self { config }
    }

    /// Detect locale from system environment — used before config is written.
    pub fn detect() -> LocaleConfig {
        let timezone = detect_timezone();
        let language = detect_language();
        let country = country_from_language(&language);
        let (date_format, units, currency) = defaults_for_country(&country);

        LocaleConfig {
            country,
            timezone,
            language,
            date_format,
            units,
            currency,
        }
    }

    /// System prompt prefix injected into every query.
    pub fn system_prompt_prefix(&self) -> String {
        let tz: Tz = self.config.timezone.parse().unwrap_or(chrono_tz::UTC);
        let now: DateTime<chrono::Utc> = chrono::Utc::now();
        let local_time = now.with_timezone(&tz);
        let offset_secs = local_time.offset().fix().local_minus_utc();
        let offset_hours = offset_secs / 3600;
        let offset_mins = (offset_secs.abs() % 3600) / 60;
        let tz_abbr = format_tz_abbr(&self.config.timezone);
        let sign = if offset_hours >= 0 { "+" } else { "-" };

        format!(
            "Current date and time: {weekday} {date}, {time} {tz_abbr} (UTC{sign}{h:02}:{m:02})\nUser location: {country}\nLanguage: {language}\n",
            weekday = local_time.format("%A"),
            date = local_time.format("%d %B %Y"),
            time = local_time.format("%H:%M"),
            tz_abbr = tz_abbr,
            sign = sign,
            h = offset_hours.unsigned_abs(),
            m = offset_mins,
            country = self.config.country,
            language = self.config.language,
        )
    }

    /// Google search `gl` parameter for localised results.
    pub fn google_gl(&self) -> String {
        self.config.country.to_lowercase()
    }
}

fn detect_timezone() -> String {
    // Try IANA timezone via env TZ, then iana-time-zone if available.
    // Fallback to UTC.
    if let Ok(tz) = std::env::var("TZ") {
        if tz.parse::<Tz>().is_ok() {
            return tz;
        }
    }
    // On Windows, localtime offset is available but IANA name requires registry.
    // We approximate from the OS local time offset.
    let offset = Local::now().offset().local_minus_utc();
    iana_approx_from_offset(offset)
}

fn detect_language() -> String {
    // LANG env var is POSIX standard; Windows uses different env vars.
    for var in &["LANG", "LANGUAGE", "LC_ALL", "LC_MESSAGES"] {
        if let Ok(val) = std::env::var(var) {
            if !val.is_empty() && val != "C" && val != "POSIX" {
                // e.g. "en_AU.UTF-8" → "en-AU"
                let clean = val.split('.').next().unwrap_or(&val).replace('_', "-");
                return clean;
            }
        }
    }
    // Windows: check USERPROFILE language via registry not feasible without winapi.
    "en-US".to_string()
}

fn country_from_language(lang: &str) -> String {
    // BCP 47: "en-AU" → "AU", "fr-FR" → "FR"
    if let Some(country) = lang.split('-').nth(1) {
        return country.to_uppercase();
    }
    "US".to_string()
}

pub fn defaults_for_country(country: &str) -> (String, String, String) {
    match country {
        "AU" => ("DD/MM/YYYY".into(), "metric".into(), "AUD".into()),
        "GB" => ("DD/MM/YYYY".into(), "metric".into(), "GBP".into()),
        "CA" => ("DD/MM/YYYY".into(), "metric".into(), "CAD".into()),
        "DE" | "FR" | "IT" | "ES" | "NL" | "BE" | "AT" | "CH" => {
            ("DD.MM.YYYY".into(), "metric".into(), "EUR".into())
        }
        "JP" => ("YYYY/MM/DD".into(), "metric".into(), "JPY".into()),
        "IN" => ("DD/MM/YYYY".into(), "metric".into(), "INR".into()),
        _ => ("MM/DD/YYYY".into(), "imperial".into(), "USD".into()),
    }
}

fn format_tz_abbr(tz_name: &str) -> String {
    // Simple: extract last component after '/' for display.
    tz_name.rsplit('/').next().unwrap_or(tz_name).replace('_', " ")
}

fn iana_approx_from_offset(offset_secs: i32) -> String {
    let hours = offset_secs / 3600;
    match hours {
        -12 => "Pacific/Apia",
        -11 => "Pacific/Niue",
        -10 => "Pacific/Honolulu",
        -9 => "America/Anchorage",
        -8 => "America/Los_Angeles",
        -7 => "America/Denver",
        -6 => "America/Chicago",
        -5 => "America/New_York",
        -4 => "America/Halifax",
        -3 => "America/Sao_Paulo",
        -2 => "Atlantic/South_Georgia",
        -1 => "Atlantic/Azores",
        0 => "UTC",
        1 => "Europe/Paris",
        2 => "Europe/Helsinki",
        3 => "Europe/Moscow",
        4 => "Asia/Dubai",
        5 => "Asia/Karachi",
        6 => "Asia/Dhaka",
        7 => "Asia/Bangkok",
        8 => "Asia/Shanghai",
        9 => "Asia/Tokyo",
        10 => "Australia/Sydney",
        11 => "Pacific/Noumea",
        12 => "Pacific/Auckland",
        _ => "UTC",
    }
    .to_string()
}
