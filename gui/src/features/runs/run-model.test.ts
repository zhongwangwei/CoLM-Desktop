import {describe,it} from 'node:test';
import assert from 'node:assert/strict';
import type {RunState} from '../../lib/types';
import {remoteSubmissionBlocked,remoteRunPermissions,reconcileRemoteRun,compatibleKernel,restoreRunView,settleRuns} from './run-model';
const run:RunState={run_id:'new',case:'/case/a',status:'running',running:true,logs:['step 1'],stages:{colm:'begin'},progress:{stage:'colm',step:1,total_steps:10}};
describe('persistent run state',()=>{
 it('restores logs, identity and progress after page remount',()=>{assert.deepEqual(restoreRunView(run,'/case/a'),{runId:'new',casePath:'/case/a',status:'running',lines:['step 1'],stage:'colm',step:1,total:10})});
 it('does not let an old completion settle a newer run',()=>{const records={'/case/a':run};assert.equal(settleRuns(records,['/case/a'],'old',true)['/case/a'],run)});
 it('does not report cancellation as completion',()=>{const cancelled={...run,running:false,status:'cancelled'};assert.equal(settleRuns({'/case/a':cancelled},['/case/a'],'new',true)['/case/a'].status,'cancelled')});
 it('settles only targets belonging to the same batch',()=>{const other={...run,run_id:'other'};const result=settleRuns({'/case/a':run,'/case/b':other},['/case/a','/case/b'],'new',true);assert.equal(result['/case/a'].running,false);assert.equal(result['/case/b'].running,true)});
 it('does not resubmit unknown remote jobs',()=>{assert.equal(remoteSubmissionBlocked({job:'123'}),true);assert.equal(remoteSubmissionBlocked({status:{state:'lost'}}),true);assert.equal(remoteSubmissionBlocked({host:'server',job:'123',status:{state:'finished',exit_code:1}}),false);assert.equal(remoteSubmissionBlocked(null),false)});
 it('selects a compatible crop kernel instead of the first kernel',()=>{const kernels=[{dir:'/default',preset:'default',macros:['SinglePoint','LULC_IGBP']},{dir:'/crop',preset:'crop',macros:['SinglePoint','LULC_IGBP','CROP']}];assert.equal(compatibleKernel(kernels,{}, {DEF_TUNING_CROP_PLANTING_DAY:'120'})?.dir,'/crop')});
});

it('pending run identity requires reconciliation before mutations or resubmission',()=>{
 const pending={host:'server',job:'new-job',pending:true,needs_reconciliation:true,status:{state:'finished',exit_code:0}};
 assert.deepEqual(remoteRunPermissions(pending),{pending:true,terminal:false,cancel:false,fetch:false});
 assert.equal(remoteSubmissionBlocked(pending),true);
 const unresolved=reconcileRemoteRun(pending,{record:null,pending:{job:'new-job'},status:{state:'unknown'}});
 assert.equal(remoteRunPermissions(unresolved).cancel,false);
 const confirmed=reconcileRemoteRun(unresolved,{record:{host:'server',job:'new-job'},pending:null,status:{state:'running'}});
 assert.equal(remoteRunPermissions(confirmed).pending,false);assert.equal(remoteRunPermissions(confirmed).cancel,true);
 assert.equal(remoteRunPermissions({...confirmed,status:{state:'finished'}}).fetch,false);
 assert.equal(remoteRunPermissions({...confirmed,status:{state:'finished',exit_code:0}}).fetch,true);
});

it('uses materialized model crop and land cover instead of stale original physics',()=>{
 const kernels=[{dir:'/default',preset:'default',macros:['SinglePoint','LULC_IGBP']},{dir:'/crop',preset:'crop',macros:['SinglePoint','LULC_IGBP','CROP']},{dir:'/usgs',preset:'usgs',macros:['SinglePoint','LULC_USGS']}];
 assert.equal(compatibleKernel(kernels,{subgrid:'USGS',crop:false},{DEF_TUNING_CROP_PLANTING_DAY:'120'})?.dir,'/usgs');
 assert.equal(compatibleKernel(kernels,{subgrid:'IGBP',crop:true},{})?.dir,'/crop');
 assert.equal(compatibleKernel(kernels,{subgrid:'IGBP',crop:false},{DEF_TUNING_CROP_PLANTING_DAY:'120'})?.dir,'/default');
});

it('keeps USGS classification independent of PFT and PC structure',()=>{
 const kernels=[{dir:'/igbp',preset:'default',macros:['SinglePoint','LULC_IGBP']},{dir:'/usgs',preset:'usgs',macros:['SinglePoint','LULC_USGS']}];
 for(const subgrid of ['PFT','PC'])assert.equal(compatibleKernel(kernels,{classification:'USGS',subgrid,crop:false})?.dir,'/usgs');
 assert.equal(compatibleKernel(kernels.slice(0,1),{classification:'USGS',subgrid:'PFT',crop:false}),undefined,'no compatible USGS kernel must not silently become IGBP');
});
