/* Tool-specific presentation of public facts. Renderers never receive arguments or raw output. */
window.AgtToolRenderers = (() => {
  const names = {harness_models:'查询可用模型',harness_session_models:'查询会话可用模型',harness_session_get:'读取执行会话',harness_session_configure:'修改会话模型',resource_list:'查看资源',resource_read:'读取资源配置',resource_write:'配置资源',agent_bind:'绑定并启用 Agent',agent_set_enabled:'调整 Agent 启用状态',task_request:'安排协作任务',task_get:'查看任务进度',message_send:'发送协作消息',local_exec:'执行本机命令'};
  const registry = new Map([
    ['command', a => ({title:a.label||'执行命令',category:'命令',symbol:'›_',note:'展开查看命令与输出。'})],
    ['files', a => ({title:a.label||'修改文件',category:'文件',symbol:'±',note:'展开查看执行器提供的文件变更。'})],
    ['framework', a => ({title:a.label||names[a.tool_name?.replace(/^framework_/, '')]||a.tool_name?.replace(/^framework_/, '')||'框架工具',category:'协作',symbol:'↗',note:'展开可按权限查看调用参数与结果。'})],
    ['mcp', a => ({title:a.label||a.tool_name||'MCP 工具',category:'MCP',symbol:'⋈',note:'展开可按权限查看外部工具参数与结果。'})],
    ['input', a => ({title:a.label||'等待引擎交互',category:'交互',symbol:'?',note:'执行器报告需要交互。'})]
  ]);
  const toolRenderers = new Map([['local_exec',registry.get('command')],['framework_local_exec',registry.get('command')]]);
  const fallback = a => ({title:a.label||a.tool_name||'工具调用',category:'工具',symbol:'·',note:'展开查看可用的调用详情。'});
  return { describe(a) { return (toolRenderers.get(a.tool_name)||registry.get(a.kind)||fallback)(a); } };
})();
