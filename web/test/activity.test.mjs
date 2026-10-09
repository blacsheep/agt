import test from 'node:test';
import assert from 'node:assert/strict';
import { activitySnapshot, publicActivities } from '../activity.mjs';

function fixture() {
  const scheduler={online:true,tasks:[{id:'task',version:2,state:'responding',principal:'agent',conversation:'chat',discussion:'round'}],discussions:[{id:'round',state:'active'}]};
  const items=[{id:'call:7',kind:'framework',tool_name:'framework_resource_list',label:'查看资源',summary:'找到 3 项资源',status:'running',started_at:10,updated_at:20,arguments:'SECRET',output:'SECRET'}];
  const record={id:'run',request:{agent:'default/agent',trigger:{conversation_id:'chat',principal:'agent'},input:'SECRET'},state:'running',progress:{task:'task',task_version:2,principal:'agent',phase:'tool_running',started_at:10,updated_at:20,preview:'draft',activities:items},execution:{engine:'codex',activity:{items,started_at:10,ended_at:30}},output:'SECRET'};
  const approvals=[{id:'approval',run_id:'run',conversation:'chat',source:'native',state:'awaiting_human',delivery:'not_sent',request:{arguments:'SECRET'},reason:'SECRET'}];
  let allowed=true;
  const reads=[];
  const run=async args=>{reads.push(args);return args[0]==='run'?[record]:args[0]==='schedule'?scheduler:approvals;};
  const tool=async(name,args,viewer)=>{assert.equal(name,'member_list');return allowed?[{principal_id:viewer,left_message_seq:null}]:[];};
  return {record,scheduler,items,approvals,reads,revoke(){allowed=false;},read:(conversation='chat',viewer='operator')=>activitySnapshot({run,tool,conversation,viewer})};
}

test('stable tool identities and safe summaries survive completion and reload without drafts', async()=>{
  const f=fixture();
  const live=(await f.read()).runs[0];
  assert.equal(live.progress.activities[0].id,'call:7');
  assert.deepEqual(live.tool_activity.approvals,[{id:'approval',source:'native',state:'awaiting_human'}]);
  assert.doesNotMatch(JSON.stringify(live),/SECRET|arguments|output/);
  f.record.state='completed';f.record.progress=null;f.items[0].status='completed';
  f.scheduler.online=false;f.scheduler.discussions[0].state='ended';
  const final=(await f.read()).runs[0];
  assert.equal(final.progress,null);
  assert.equal(final.tool_activity.items[0].id,'call:7');
  assert.equal(final.tool_activity.items[0].status,'completed');
  assert.equal(final.tool_activity.ended_at,30);
  assert.doesNotMatch(JSON.stringify(final),/draft|SECRET/);
  assert.deepEqual((await f.read()).runs[0].tool_activity,final.tool_activity);
});

test('finished work cannot bypass current membership, selected conversation or root run scope',async()=>{
  const f=fixture();f.record.state='failed';f.record.progress=null;
  assert.equal((await f.read()).runs[0].tool_activity.items[0].status,'unknown');
  assert.equal((await f.read('another')).runs[0].tool_activity,null);
  assert.equal((await f.read(null)).runs[0].tool_activity,null);
  f.record.parent_run_id='parent';assert.equal((await f.read()).runs[0].tool_activity,null);
  f.record.parent_run_id=null;f.revoke();assert.equal((await f.read()).runs[0].tool_activity,null);
});

test('stale running snapshots are not mistaken for completed history or current approval',async()=>{
  const f=fixture();
  for(const change of [()=>f.scheduler.online=false,()=>{f.scheduler.online=true;f.record.progress.task_version=1;},()=>{f.record.progress.task_version=2;f.scheduler.discussions[0].state='ended';}]){
    change();const row=(await f.read()).runs[0];assert.equal(row.progress,null);assert.equal(row.tool_activity,null);
  }
  f.scheduler.discussions[0].state='paused';assert.equal((await f.read()).runs[0].tool_activity.items[0].id,'call:7');
  f.approvals[0].conversation='private';assert.deepEqual((await f.read()).runs[0].tool_activity.approvals,[]);
});

test('activity projection bounds history, deduplicates IDs, strips raw engine identities and unknown fields',()=>{
  const value=publicActivities([{id:'SECRET command path',kind:'command',status:'running',started_at:1,updated_at:2,command:'SECRET'},
    {id:'call:2',kind:'mcp',tool_name:'search',status:'running'},
    {id:'call:2',kind:'mcp',tool_name:'search',status:'failed'},
    {id:'call:3',kind:'private',status:'completed'}],true);
  assert.equal(value.length,2);assert.equal(value[0].status,'unknown');assert.equal(value[1].status,'failed');
  assert.doesNotMatch(JSON.stringify(value),/SECRET|private/);
  assert.equal(publicActivities(Array.from({length:45},(_,i)=>({id:`call:${i}`,kind:'command',status:'completed'}))).length,40);
});
