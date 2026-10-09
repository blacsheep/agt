use super::*;

pub(super) fn write(
    h: &Path,
    category: &CategoryName,
    name: &str,
    create: bool,
    data: Value,
) -> Result<()> {
    let document = McpDocument::parse(Path::new("mcp.json"), &serde_json::to_string(&data)?)?;
    if document.name.as_str() != name {
        return Err(crate::ToolError::invalid(
            Some("data.mcpServers"),
            "MCP server name must match the resource name.",
        )
        .into());
    }
    let plan = if create {
        McpWritePlan::create(document)
    } else {
        McpWritePlan::replace(document)
    };
    McpStore::new(h).apply_batch_in(category, vec![plan])?;
    Ok(())
}
