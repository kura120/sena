import React, { useState } from "react";
import { IconThought, IconChevronRight, IconChevronDown } from "../../components/icons";
import { STRINGS } from "../../constants/strings";
import "./ThoughtBlock.css";

interface ThoughtBlockProps {
  content: string;
  isStreaming?: boolean;
}

export const ThoughtBlock: React.FC<ThoughtBlockProps> = ({ content, isStreaming = false }) => {
  const [expanded, setExpanded] = useState(false);
  
  // Estimate duration: ~50 chars/sec, minimum 1s
  const duration = Math.max(1, Math.round(content.length / 50));
  
  const toggleExpanded = () => {
    setExpanded(!expanded);
  };

  return (
    <div className="thought-block">
      <div className="thought-header" onClick={toggleExpanded}>
        <IconThought size={14} className="icon-thought" />
        <span className="duration-text">
          {isStreaming 
            ? STRINGS.COT_THINKING 
            : `${STRINGS.COT_THOUGHT_FOR} ${duration}${STRINGS.COT_SECONDS}`
          }
        </span>
        {expanded ? (
          <IconChevronDown size={14} />
        ) : (
          <IconChevronRight size={14} />
        )}
      </div>
      
      {expanded && (
        <div className="thought-content">
          {content}
        </div>
      )}
    </div>
  );
};
