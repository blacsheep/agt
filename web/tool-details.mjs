// Raw payloads are fetched separately from public activity snapshots.
export async function toolDetail({run, tool, operator, viewer, conversation, id, activity}) {
  const record = await run(['run', 'info', id]);
  const trigger = record.request?.trigger;
  if (record.parent_run_id || trigger?.conversation_id !== conversation || !conversation ||
      (viewer !== operator && viewer !== trigger.principal)) return undefined;
  const members = await tool('member_list', {conversation}, viewer).catch(() => []);
  if (!members.some(m => m.principal_id === viewer && m.left_message_seq == null)) return undefined;
  return run(['run', 'tool-detail', id, '--activity', activity]);
}
