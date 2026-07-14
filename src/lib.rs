use serde_json::Value;
use thiserror::Error;

pub const QUERY_URL: &str = "https://mcard.sdu.edu.cn/charge/feeitem/getThirdData";

pub const BUILDINGS: &[(&str, &str)] = &[
    ("S1", "1503975832"),
    ("S2", "1503975890"),
    ("S5", "1503975967"),
    ("S6", "1503975980"),
    ("S7", "1503975988"),
    ("S8", "1503975995"),
    ("S9", "1503976004"),
    ("S10", "1503976037"),
    ("S11", "1599193777"),
    ("B1", "1661835249"),
    ("B2", "1661835256"),
    ("B5", "1661835273"),
    ("B9", "1693031698"),
    ("B10", "1693031710"),
];

#[derive(Debug, Error, PartialEq)]
pub enum QueryError {
    #[error("令牌无效或已失效：{0}")]
    InvalidAuth(String),
    #[error("查询接口返回 HTTP {0}")]
    Http(u16),
    #[error("查询响应格式异常：{0}")]
    InvalidResponse(String),
}

pub fn building_id(building: &str) -> Option<String> {
    let normalized = building.trim().to_ascii_uppercase();
    BUILDINGS
        .iter()
        .find(|(name, _)| *name == normalized)
        .map(|(name, timestamp)| format!("{timestamp}&{name}"))
}

pub fn query_form(building: &str, room: &str) -> Result<Vec<(&'static str, String)>, QueryError> {
    let building = building_id(building).ok_or_else(|| {
        QueryError::InvalidResponse(format!("不支持的宿舍楼：{}", building.trim()))
    })?;
    let room = room.trim();
    if room.is_empty() {
        return Err(QueryError::InvalidResponse("宿舍号不能为空".to_string()));
    }

    Ok(vec![
        ("feeitemid", "410".to_string()),
        ("type", "IEC".to_string()),
        ("level", "3".to_string()),
        ("campus", "青岛校区&青岛校区".to_string()),
        ("building", building),
        ("room", room.to_string()),
    ])
}

pub fn parse_query_response(status: u16, body: &str) -> Result<f64, QueryError> {
    if matches!(status, 401 | 403) {
        return Err(QueryError::InvalidAuth(api_message(body)));
    }
    if !(200..300).contains(&status) {
        return Err(QueryError::Http(status));
    }

    let value: Value = serde_json::from_str(body)
        .map_err(|error| QueryError::InvalidResponse(error.to_string()))?;
    let message = value
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let code = value.get("code").and_then(Value::as_i64);
    if code == Some(401)
        || code == Some(403)
        || ["令牌", "鉴权", "未登录", "登录失效"]
            .iter()
            .any(|word| message.contains(word))
    {
        return Err(QueryError::InvalidAuth(message.to_string()));
    }

    let info = value
        .pointer("/map/showData/信息")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            QueryError::InvalidResponse(if message.is_empty() {
                "缺少 map.showData.信息".to_string()
            } else {
                message.to_string()
            })
        })?;

    parse_electricity(info)
}

fn api_message(body: &str) -> String {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|value| value.get("message")?.as_str().map(str::to_string))
        .filter(|message| !message.is_empty())
        .unwrap_or_else(|| "鉴权失败".to_string())
}

fn parse_electricity(info: &str) -> Result<f64, QueryError> {
    let candidate = info
        .rsplit_once('：')
        .or_else(|| info.rsplit_once(':'))
        .map(|(_, value)| value)
        .unwrap_or(info)
        .trim()
        .trim_end_matches('度')
        .trim();

    candidate
        .parse::<f64>()
        .map_err(|_| QueryError::InvalidResponse(format!("无法从“{info}”中读取电量数值")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_known_building() {
        assert_eq!(building_id("s10").as_deref(), Some("1503976037&S10"));
        assert_eq!(building_id("T1"), None);
    }

    #[test]
    fn parses_reference_response_shape() {
        let body = r#"{"map":{"showData":{"信息":"剩余电量：12.50度"}}}"#;
        assert_eq!(parse_query_response(200, body), Ok(12.5));
    }

    #[test]
    fn identifies_expired_token() {
        let body = r#"{"code":401,"message":"缺失令牌,鉴权失败"}"#;
        assert!(matches!(
            parse_query_response(401, body),
            Err(QueryError::InvalidAuth(_))
        ));
    }

    #[test]
    fn rejects_unexpected_success_response() {
        let body = r#"{"code":500,"message":"房间不存在"}"#;
        assert!(matches!(
            parse_query_response(200, body),
            Err(QueryError::InvalidResponse(_))
        ));
    }
}
