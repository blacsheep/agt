/* Tool-specific presentation of public facts. Renderers never receive arguments or raw output. */
window.AgtToolRenderers = (() => {
  const names = {harness_models:$t('查询可用模型'),harness_session_models:$t('查询会话可用模型'),harness_session_get:$t('读取执行会话'),harness_session_configure:$t('修改会话模型'),resource_list:$t('查看资源'),resource_read:$t('读取资源配置'),resource_write:$t('配置资源'),agent_bind:$t('绑定并启用 Agent'),agent_set_enabled:$t('调整 Agent 启用状态'),task_request:$t('安排协作任务'),task_get:$t('查看任务进度'),message_send:$t('发送协作消息'),local_exec:$t('执行本机命令')};
  const registry = new Map([
    ['command', a => ({title:(a.label&&$t(a.label))||$t('执行命令'),category:$t('命令'),symbol:'›_',note:$t('展开查看命令与输出。')})],
    ['files', a => ({title:(a.label&&$t(a.label))||$t('修改文件'),category:$t('文件'),symbol:'±',note:$t('展开查看执行器提供的文件变更。')})],
    ['framework', a => ({title:(a.label&&$t(a.label))||names[a.tool_name?.replace(/^framework_/, '')]||a.tool_name?.replace(/^framework_/, '')||$t('框架工具'),category:$t('协作'),symbol:'↗',note:$t('展开可按权限查看调用参数与结果。')})],
    ['mcp', a => ({title:(a.label&&$t(a.label))||a.tool_name||$t('MCP 工具'),category:'MCP',symbol:'⋈',note:$t('展开可按权限查看外部工具参数与结果。')})],
    ['input', a => ({title:(a.label&&$t(a.label))||$t('等待引擎交互'),category:$t('交互'),symbol:'?',note:$t('执行器报告需要交互。')})]
  ]);
  const toolRenderers = new Map([['local_exec',registry.get('command')],['framework_local_exec',registry.get('command')]]);
  const fallback = a => ({title:(a.label&&$t(a.label))||a.tool_name||$t('工具调用'),category:$t('工具'),symbol:'·',note:$t('展开查看可用的调用详情。')});
  return { describe(a) { return (toolRenderers.get(a.tool_name)||registry.get(a.kind)||fallback)(a); } };
})();
