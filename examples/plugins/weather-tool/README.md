# Weather Tool

Example bsv-worm tool plugin demonstrating the `Tool` trait.

## What it does

Looks up current weather conditions for a given city. Returns temperature, conditions, humidity, and wind speed. Uses mock data -- a real implementation would call a weather API.

## Usage

```rust
use weather_tool::WeatherTool;
use bsv_worm_sdk::tool::{Tool, ToolInput};
use serde_json::json;

let tool = WeatherTool::new();
let input = ToolInput::new(json!({ "city": "Austin", "units": "celsius" }));
let result = tool.execute(input).await.unwrap();
println!("{}", result.output);
// "Weather in Austin: 17C | Partly Cloudy | Humidity: 46% | Wind: 11 mph"
```

## Parameters

| Name | Type | Required | Description |
|------|------|----------|-------------|
| `city` | string | yes | City name |
| `units` | string | no | "fahrenheit" (default) or "celsius" |

## Testing

```bash
cargo test -p weather-tool
```
