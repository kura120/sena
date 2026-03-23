import { STRINGS } from "../../constants/strings";
import { CAPABILITY_COLORS } from "../../constants/colors";
import { IconCheck, IconWarning, IconClose } from "../icons";
import type { CapabilityBreakdown, CapabilityItem } from "../../types";
import "./CapabilityDetail.css";

interface CapabilityDetailProps {
  capabilities: CapabilityBreakdown;
}

export function CapabilityDetail({ capabilities }: CapabilityDetailProps) {
  const hasGranted = capabilities.granted.length > 0;
  const hasDegraded = capabilities.degraded.length > 0;
  const hasDenied = capabilities.denied.length > 0;

  if (!hasGranted && !hasDegraded && !hasDenied) {
    return (
      <div className="capability-detail">
        <div className="capability-empty">{STRINGS.CAPABILITY_ALL_OPERATIONAL}</div>
      </div>
    );
  }

  return (
    <div className="capability-detail">
      {hasGranted && (
        <div className="capability-section">
          <div className="capability-section-header" style={{ color: CAPABILITY_COLORS.granted }}>
            <IconCheck size={12} />
            <span>{STRINGS.CAPABILITY_GRANTED}</span>
            <span className="capability-count">({capabilities.granted.length})</span>
          </div>
          <div className="capability-list">
            {capabilities.granted.map((item, idx) => (
              <CapabilityItemRow key={idx} item={item} />
            ))}
          </div>
        </div>
      )}

      {hasDegraded && (
        <div className="capability-section">
          <div className="capability-section-header" style={{ color: CAPABILITY_COLORS.degraded }}>
            <IconWarning size={12} />
            <span>{STRINGS.CAPABILITY_DEGRADED}</span>
            <span className="capability-count">({capabilities.degraded.length})</span>
          </div>
          <div className="capability-list">
            {capabilities.degraded.map((item, idx) => (
              <CapabilityItemRow key={idx} item={item} />
            ))}
          </div>
        </div>
      )}

      {hasDenied && (
        <div className="capability-section">
          <div className="capability-section-header" style={{ color: CAPABILITY_COLORS.denied }}>
            <IconClose size={12} />
            <span>{STRINGS.CAPABILITY_DENIED}</span>
            <span className="capability-count">({capabilities.denied.length})</span>
          </div>
          <div className="capability-list">
            {capabilities.denied.map((item, idx) => (
              <CapabilityItemRow key={idx} item={item} />
            ))}
          </div>
        </div>
      )}
    </div>
  );
}

function CapabilityItemRow({ item }: { item: CapabilityItem }) {
  return (
    <div className="capability-item">
      <div className="capability-item-label">• {item.label}</div>
      {item.reason && <div className="capability-item-reason">{item.reason}</div>}
    </div>
  );
}
