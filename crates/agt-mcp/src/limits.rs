#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct McpLimits {
    pub max_frame_bytes: usize,
    pub max_http_header_bytes: usize,
    pub max_http_body_bytes: usize,
    pub max_sse_event_bytes: usize,
    pub max_stderr_bytes: usize,
    pub max_list_pages: usize,
    pub max_tools_per_page: usize,
    pub max_tools_total: usize,
    pub max_tool_name_bytes: usize,
    pub max_description_bytes: usize,
    pub max_input_schema_bytes: usize,
    pub max_output_schema_bytes: usize,
    pub max_tool_metadata_bytes: usize,
    pub max_call_result_bytes: usize,
    pub max_queued_events: usize,
}

impl Default for McpLimits {
    fn default() -> Self {
        Self {
            max_frame_bytes: 8 * 1024 * 1024,
            max_http_header_bytes: 64 * 1024,
            max_http_body_bytes: 8 * 1024 * 1024,
            max_sse_event_bytes: 8 * 1024 * 1024,
            max_stderr_bytes: 8 * 1024,
            max_list_pages: 1_000,
            max_tools_per_page: 256,
            max_tools_total: 1_024,
            max_tool_name_bytes: 128,
            max_description_bytes: 16 * 1024,
            max_input_schema_bytes: 128 * 1024,
            max_output_schema_bytes: 128 * 1024,
            max_tool_metadata_bytes: 16 * 1024 * 1024,
            max_call_result_bytes: 4 * 1024 * 1024,
            max_queued_events: 256,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_the_pinned_mcp_resource_contract() {
        assert_eq!(
            McpLimits::default(),
            McpLimits {
                max_frame_bytes: 8 * 1024 * 1024,
                max_http_header_bytes: 64 * 1024,
                max_http_body_bytes: 8 * 1024 * 1024,
                max_sse_event_bytes: 8 * 1024 * 1024,
                max_stderr_bytes: 8 * 1024,
                max_list_pages: 1_000,
                max_tools_per_page: 256,
                max_tools_total: 1_024,
                max_tool_name_bytes: 128,
                max_description_bytes: 16 * 1024,
                max_input_schema_bytes: 128 * 1024,
                max_output_schema_bytes: 128 * 1024,
                max_tool_metadata_bytes: 16 * 1024 * 1024,
                max_call_result_bytes: 4 * 1024 * 1024,
                max_queued_events: 256,
            }
        );
    }
}
