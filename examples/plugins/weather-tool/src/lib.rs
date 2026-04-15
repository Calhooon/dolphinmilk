//! Weather Tool — example bsv-worm tool plugin.
//!
//! Demonstrates how to implement the `Tool` trait from `bsv-worm-sdk`.
//! Returns mock weather data for any city. In a real plugin, this would
//! call a weather API.

use async_trait::async_trait;
use bsv_worm_sdk::tool::{Tool, ToolError, ToolInput, ToolResult};
use serde_json::json;

/// A mock weather lookup tool.
///
/// Accepts a city name and returns mock weather data including temperature,
/// conditions, humidity, and wind speed.
pub struct WeatherTool;

impl WeatherTool {
    /// Create a new WeatherTool instance.
    pub fn new() -> Self {
        Self
    }

    /// Generate mock weather data for a city.
    fn mock_weather(city: &str) -> serde_json::Value {
        // Deterministic mock data based on city name length for testability.
        let temp_f = 60 + (city.len() % 30) as i32;
        let temp_c = (temp_f - 32) * 5 / 9;
        let humidity = 40 + (city.len() % 50) as i32;
        let wind_mph = 5 + (city.len() % 20) as i32;

        let conditions = match city.len() % 4 {
            0 => "Sunny",
            1 => "Partly Cloudy",
            2 => "Overcast",
            _ => "Light Rain",
        };

        json!({
            "city": city,
            "temperature": {
                "fahrenheit": temp_f,
                "celsius": temp_c,
            },
            "conditions": conditions,
            "humidity_percent": humidity,
            "wind_mph": wind_mph,
        })
    }
}

impl Default for WeatherTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for WeatherTool {
    fn name(&self) -> &str {
        "weather_lookup"
    }

    fn description(&self) -> &str {
        "Look up current weather conditions for a city. Returns temperature, conditions, humidity, and wind speed."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "city": {
                    "type": "string",
                    "description": "City name (e.g. 'Austin', 'London', 'Tokyo')"
                },
                "units": {
                    "type": "string",
                    "enum": ["fahrenheit", "celsius"],
                    "description": "Temperature units (default: fahrenheit)"
                }
            },
            "required": ["city"]
        })
    }

    fn category(&self) -> &str {
        "custom"
    }

    async fn execute(&self, input: ToolInput) -> Result<ToolResult, ToolError> {
        let city = input
            .get_str("city")
            .ok_or_else(|| ToolError::MissingParameter("city".into()))?;

        if city.trim().is_empty() {
            return Err(ToolError::InvalidInput("city cannot be empty".into()));
        }

        let weather = Self::mock_weather(city);

        let units = input.get_str("units").unwrap_or("fahrenheit");
        let temp = if units == "celsius" {
            format!(
                "{}C",
                weather["temperature"]["celsius"].as_i64().unwrap_or(0)
            )
        } else {
            format!(
                "{}F",
                weather["temperature"]["fahrenheit"].as_i64().unwrap_or(0)
            )
        };

        let output = format!(
            "Weather in {}: {} | {} | Humidity: {}% | Wind: {} mph",
            city,
            temp,
            weather["conditions"].as_str().unwrap_or("Unknown"),
            weather["humidity_percent"].as_i64().unwrap_or(0),
            weather["wind_mph"].as_i64().unwrap_or(0),
        );

        Ok(ToolResult::text(output))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_weather_lookup() {
        let tool = WeatherTool::new();
        let input = ToolInput::new(json!({ "city": "Austin" }));
        let result = tool.execute(input).await.unwrap();

        assert!(result.success);
        assert!(result.output.contains("Austin"));
        assert!(result.output.contains("F"));
        assert!(result.output.contains("Humidity"));
    }

    #[tokio::test]
    async fn test_weather_celsius() {
        let tool = WeatherTool::new();
        let input = ToolInput::new(json!({ "city": "London", "units": "celsius" }));
        let result = tool.execute(input).await.unwrap();

        assert!(result.success);
        assert!(result.output.contains("London"));
        assert!(result.output.contains("C"));
    }

    #[tokio::test]
    async fn test_weather_missing_city() {
        let tool = WeatherTool::new();
        let input = ToolInput::new(json!({}));
        let result = tool.execute(input).await;

        assert!(result.is_err());
        match result.unwrap_err() {
            ToolError::MissingParameter(param) => assert_eq!(param, "city"),
            other => panic!("Expected MissingParameter, got: {other}"),
        }
    }

    #[tokio::test]
    async fn test_weather_empty_city() {
        let tool = WeatherTool::new();
        let input = ToolInput::new(json!({ "city": "  " }));
        let result = tool.execute(input).await;

        assert!(result.is_err());
        match result.unwrap_err() {
            ToolError::InvalidInput(msg) => assert!(msg.contains("empty")),
            other => panic!("Expected InvalidInput, got: {other}"),
        }
    }

    #[test]
    fn test_tool_trait_metadata() {
        let tool = WeatherTool::new();
        assert_eq!(tool.name(), "weather_lookup");
        assert_eq!(tool.category(), "custom");
        assert!(!tool.description().is_empty());

        let schema = tool.parameters_schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["city"].is_object());
    }
}
