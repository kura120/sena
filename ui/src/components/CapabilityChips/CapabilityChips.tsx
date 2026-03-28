import { STRINGS } from "../../constants/strings";
import { IconCheck, IconWarning, IconClose } from "../icons";
import type { CapabilityBreakdown } from "../../types";
import "./CapabilityChips.css";

interface CapabilityChipsProps {
  capabilities: CapabilityBreakdown;
}

function formatSummary(capabilities: CapabilityBreakdown): string {
  const total =
    capabilities.granted.length +
    capabilities.degraded.length +
    capabilities.denied.length;

  let summary = STRINGS.VERBOSE_CAPABILITIES_SUMMARY.replace("{total}", String(total));
  if (capabilities.degraded.length > 0) {
    summary += STRINGS.VERBOSE_DEGRADED_SUFFIX.replace(
      "{count}",
      String(capabilities.degraded.length),
    );
  }
  return summary;
}

export function CapabilityChips({ capabilities }: CapabilityChipsProps) {
  const hasGranted = capabilities.granted.length > 0;
  const hasDegraded = capabilities.degraded.length > 0;
  const hasDenied = capabilities.denied.length > 0;

  if (!hasGranted && !hasDegraded && !hasDenied) {
    return (
      <div className="capability-chips">
        <div className="capability-chips-empty">{STRINGS.VERBOSE_NO_DETAILS}</div>
      </div>
    );
  }

  return (
    <div className="capability-chips">
      <div className="capability-chips-summary">{formatSummary(capabilities)}</div>
      <div className="capability-chips-group">
        {capabilities.granted.map((item, idx) => (
          <span key={`g-${idx}`} className="capability-chip capability-chip--granted" title={item.reason || undefined}>
            <span className="capability-chip-icon"><IconCheck size={10} /></span>
            {item.label}
          </span>
        ))}
        {capabilities.degraded.map((item, idx) => (
          <span key={`d-${idx}`} className="capability-chip capability-chip--degraded" title={item.reason || undefined}>
            <span className="capability-chip-icon"><IconWarning size={10} /></span>
            {item.label}
          </span>
        ))}
        {capabilities.denied.map((item, idx) => (
          <span key={`x-${idx}`} className="capability-chip capability-chip--denied" title={item.reason || undefined}>
            <span className="capability-chip-icon"><IconClose size={10} /></span>
            {item.label}
          </span>
        ))}
      </div>
    </div>
  );
}
