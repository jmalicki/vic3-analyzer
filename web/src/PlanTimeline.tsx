import type { PlanResult, PlanAction } from './types'

interface Props {
  plan: PlanResult
}

function formatAction(action: PlanAction): string {
  if ('QueueTech' in action) return `Queue Tech: ${action.QueueTech.tech}`
  if ('QueueBuildingLevel' in action) return `Queue Building: ${action.QueueBuildingLevel.building}`
  if ('QueueInterest' in action) return `Declare Interest: ${action.QueueInterest.kind} in ${action.QueueInterest.id}`
  if ('QueueHireMilitary' in action) return `Hire Military: ${action.QueueHireMilitary.building}`
  if ('QueueLaw' in action) return `Enact Law: ${action.QueueLaw.law}`
  if ('SwitchPm' in action) return `Switch Production Methods on Building ${action.SwitchPm.building_id}: ${action.SwitchPm.methods.join(', ')}`
  if ('AdjustTax' in action) return `Adjust Tax: Delta ${action.AdjustTax.delta}`
  if ('WaitForEvent' in action) {
    const ev = action.WaitForEvent.event
    if ('TechCompleted' in ev) return `Wait for Tech: ${ev.TechCompleted.tech}`
    if ('BuildingCompleted' in ev) return `Wait for Building: ${ev.BuildingCompleted.building}`
    if ('InterestDeclared' in ev) return `Wait for Interest: ${ev.InterestDeclared.kind} in ${ev.InterestDeclared.id}`
    if ('HireCompleted' in ev) return `Wait for Hire: ${ev.HireCompleted.building}`
    if ('LawEnacted' in ev) return `Wait for Law: ${ev.LawEnacted.law}`
  }
  return JSON.stringify(action)
}

export function PlanTimeline({ plan }: Props) {
  return (
    <div className="plan-timeline">
      <div className="plan-timeline-header">
        <h3>Execution Plan</h3>
        <p>Total time: <strong>{plan.day_cost} days</strong></p>
        {plan.limitations.length > 0 && (
          <div className="plan-limitations">
            <strong>Limitations:</strong>
            <ul>
              {plan.limitations.map((l, i) => <li key={i}>{l}</li>)}
            </ul>
          </div>
        )}
      </div>
      <div className="plan-timeline-steps">
        {plan.actions.map((step, i) => (
          <div className="plan-timeline-step" key={i}>
            <div className="plan-timeline-day">Day {step.day}</div>
            <div className="plan-timeline-action">{formatAction(step.action)}</div>
          </div>
        ))}
        {plan.actions.length === 0 && (
          <div className="plan-timeline-step empty">No steps required.</div>
        )}
      </div>
    </div>
  )
}
