import {Button} from '../../components/ui';
import {simulationSteps,type SimulationStep} from './simulation-model';
export default function SimulationSteps({step,en,disabled=false,onChange}:{step:SimulationStep;en:boolean;disabled?:boolean;onChange:(step:SimulationStep)=>void}){
 return <nav className="simulation-steps" aria-label={en?'Simulation steps':'模拟步骤'}>{simulationSteps.map((label,i)=><Button key={label[0]} variant="ghost" className={`${step===i+1?'current':''} ${step>i+1?'visited':''}`} aria-current={step===i+1?'step':undefined} disabled={disabled||i+1>step} onClick={()=>onChange(i+1 as SimulationStep)}><span className="step-marker" aria-hidden="true">{step>i+1?'✓':i+1}</span><span>{label[en?1:0]}</span></Button>)}</nav>;
}
