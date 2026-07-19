use chrono::Utc;
use chrono_tz::Asia::Shanghai;
use lettre::message::header::ContentType;
use lettre::transport::smtp::authentication::Credentials;
use lettre::{Message, SmtpTransport, Transport};
use sdu_equery::{parse_query_response, query_form, QueryError, QUERY_URL};
use std::collections::BTreeMap;
use std::env;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::time::sleep;

type AppResult<T> = Result<T, Box<dyn Error>>;

const QUERY_MAX_ATTEMPTS: u32 = 3;

#[derive(Debug)]
struct Config {
    auth: String,
    building: String,
    room: String,
    threshold: f64,
    history_csv: PathBuf,
}

#[derive(Debug)]
struct MailConfig {
    host: String,
    port: u16,
    username: String,
    password: String,
    from: String,
    recipients: Vec<String>,
}

impl Config {
    fn from_env() -> AppResult<Self> {
        let threshold = required("ALERT_THRESHOLD")?.parse::<f64>()?;
        if !threshold.is_finite() {
            return Err("ALERT_THRESHOLD 必须是有限数值".into());
        }
        Ok(Self {
            auth: required("SDU_AUTH")?,
            building: required("SDU_BUILDING")?,
            room: required("SDU_ROOM")?,
            threshold,
            history_csv: env::var("HISTORY_CSV")
                .map(PathBuf::from)
                .unwrap_or_else(|_| PathBuf::from("history.csv")),
        })
    }
}

impl MailConfig {
    fn from_env() -> AppResult<Self> {
        let username = required("SMTP_USERNAME")?;
        let recipients = required("MAIL_TO")?
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .collect::<Vec<_>>();
        if recipients.is_empty() {
            return Err("MAIL_TO 至少需要一个收件地址".into());
        }
        Ok(Self {
            host: required("SMTP_HOST")?,
            port: required("SMTP_PORT")?.parse::<u16>()?,
            password: required("SMTP_PASSWORD")?,
            from: env::var("MAIL_FROM").unwrap_or_else(|_| username.clone()),
            username,
            recipients,
        })
    }
}

#[tokio::main]
async fn main() -> AppResult<()> {
    let config = Config::from_env()?;
    let now = Utc::now().with_timezone(&Shanghai);

    let electricity = match query(&config).await {
        Ok(value) => value,
        Err(QueryError::InvalidAuth(message)) => {
            let subject = "[宿舍电量] 查询令牌已失效";
            let body = format!(
                "北京时间 {} 查询 {} {} 时鉴权失败：{}。请更新仓库 Secret SDU_AUTH。",
                now.format("%Y-%m-%d %H:%M:%S"),
                config.building,
                config.room,
                message
            );
            send_mail(subject, &body)?;
            return Err(QueryError::InvalidAuth(message).into());
        }
        Err(error) => return Err(error.into()),
    };

    let date = now.format("%Y-%m-%d").to_string();
    let building = config.building.trim().to_ascii_uppercase();
    let room = config.room.trim();
    write_history(&config.history_csv, &date, electricity)?;

    println!("{} {} {}：{}", date, building, room, electricity);

    if electricity < config.threshold {
        let subject = "[宿舍电量] 余额低于阈值";
        let body = format!(
            "{} {} 当前剩余电量为 {}，已低于设定阈值 {}。查询时间：{}（北京时间）。",
            building,
            room,
            electricity,
            config.threshold,
            now.format("%Y-%m-%d %H:%M:%S")
        );
        send_mail(subject, &body)?;
    }

    Ok(())
}

async fn query(config: &Config) -> Result<f64, QueryError> {
    let form = query_form(&config.building, &config.room)?;
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(25))
        .build()
        .map_err(|error| QueryError::InvalidResponse(error.to_string()))?;

    for attempt in 1..=QUERY_MAX_ATTEMPTS {
        let result = async {
            let response = client
                .post(QUERY_URL)
                .header("Synjones-Auth", &config.auth)
                .header("Accept", "application/json, text/plain, */*")
                .form(&form)
                .send()
                .await?;
            let status = response.status().as_u16();
            let body = response.text().await?;
            Ok::<_, reqwest::Error>((status, body))
        }
        .await;

        match result {
            Ok((status, _)) if should_retry_status(status) && attempt < QUERY_MAX_ATTEMPTS => {
                eprintln!(
                    "查询接口返回 HTTP {status}，准备进行第 {} 次尝试",
                    attempt + 1
                );
            }
            Ok((status, body)) => return parse_query_response(status, &body),
            Err(error) if attempt < QUERY_MAX_ATTEMPTS => {
                eprintln!(
                    "查询请求失败（第 {attempt}/{QUERY_MAX_ATTEMPTS} 次）：{error}，即将重试"
                );
            }
            Err(error) => return Err(QueryError::InvalidResponse(error.to_string())),
        }

        sleep(Duration::from_secs(1_u64 << attempt)).await;
    }

    unreachable!("查询重试循环至少执行一次")
}

fn should_retry_status(status: u16) -> bool {
    matches!(status, 408 | 425 | 429 | 500..=599)
}

fn write_history(path: &Path, date: &str, electricity: f64) -> AppResult<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }

    let mut rows = BTreeMap::<String, String>::new();
    if path.exists() {
        let mut reader = csv::Reader::from_path(path)?;
        for record in reader.records() {
            let record = record?;
            if let (Some(date), Some(electricity)) = (record.get(0), record.get(1)) {
                rows.insert(date.to_string(), electricity.to_string());
            }
        }
    }
    rows.insert(date.to_string(), electricity.to_string());

    let mut writer = csv::Writer::from_path(path)?;
    writer.write_record(["date", "electricity"])?;
    for (date, electricity) in rows {
        writer.write_record([date, electricity])?;
    }
    writer.flush()?;
    Ok(())
}

fn send_mail(subject: &str, body: &str) -> AppResult<()> {
    let config = MailConfig::from_env()?;
    let mut builder = Message::builder()
        .from(config.from.parse()?)
        .subject(subject)
        .header(ContentType::TEXT_PLAIN);
    for recipient in &config.recipients {
        builder = builder.to(recipient.parse()?);
    }
    let message = builder.body(body.to_string())?;
    let credentials = Credentials::new(config.username, config.password);
    let mailer = if config.port == 465 {
        SmtpTransport::relay(&config.host)?
            .port(config.port)
            .credentials(credentials)
            .build()
    } else {
        SmtpTransport::starttls_relay(&config.host)?
            .port(config.port)
            .credentials(credentials)
            .build()
    };
    mailer.send(&message)?;
    println!("告警邮件已发送给 {} 个收件地址", config.recipients.len());
    Ok(())
}

fn required(name: &str) -> AppResult<String> {
    let value = env::var(name).map_err(|_| format!("缺少环境变量 {name}"))?;
    if value.trim().is_empty() {
        return Err(format!("环境变量 {name} 不能为空").into());
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_has_exactly_two_columns_and_replaces_same_day() {
        let path = env::temp_dir().join(format!(
            "sdu-equery-history-{}-{}.csv",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        write_history(&path, "2026-07-14", 12.5).unwrap();
        write_history(&path, "2026-07-14", 11.25).unwrap();

        let content = fs::read_to_string(&path).unwrap();
        assert_eq!(content, "date,electricity\n2026-07-14,11.25\n");
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn history_keeps_distinct_dates_without_retention_limit() {
        let path = env::temp_dir().join(format!(
            "sdu-equery-history-all-{}-{}.csv",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        write_history(&path, "2025-01-01", 20.0).unwrap();
        write_history(&path, "2026-07-15", 10.0).unwrap();

        let content = fs::read_to_string(&path).unwrap();
        assert_eq!(
            content,
            "date,electricity\n2025-01-01,20\n2026-07-15,10\n"
        );
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn retries_only_transient_http_statuses() {
        for status in [408, 425, 429, 500, 502, 599] {
            assert!(should_retry_status(status));
        }
        for status in [200, 400, 401, 403, 404] {
            assert!(!should_retry_status(status));
        }
    }
}
