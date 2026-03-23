import React, { useState, useEffect } from 'react';
import './SettingsField.css';
import { RestartBadge } from './RestartBadge';
import { Dropdown } from '../Dropdown/Dropdown';

interface SettingsFieldProps {
  label: string;
  description?: string;
  value: any;
  onChange: (value: any) => void;
  type: 'slider' | 'toggle' | 'input' | 'dropdown' | 'readonly' | 'array-input';
  min?: number;
  max?: number;
  step?: number;
  options?: { label: string; value: string }[];
  unit?: string;
  restartRequired?: boolean;
  subsystem?: string;
  error?: string;
  displayTransform?: (val: number) => string;
}

export function SettingsField({
  label,
  description,
  value,
  onChange,
  type,
  min,
  max,
  step,
  options,
  unit,
  restartRequired,
  subsystem,
  error,
  displayTransform
}: SettingsFieldProps) {
  
  // Local state for array-input to allow typing freely
  const [arrayInputVal, setArrayInputVal] = useState('');

  // Sync array input when value prop changes
  useEffect(() => {
    if (type === 'array-input' && Array.isArray(value)) {
      setArrayInputVal(value.join(', '));
    }
  }, [value, type]);

  const handleInputChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    let newValue: any = e.target.value;
    if (type === 'input') {
        if (newValue === '') {
            onChange(''); 
            return;
        }
        const num = parseFloat(newValue);
        if (!isNaN(num)) {
            newValue = num;
        }
    }
    onChange(newValue);
  };

  const handleToggleChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    onChange(e.target.checked);
  };

  const handleSliderChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    onChange(parseFloat(e.target.value));
  };
  
  const handleArrayChange = (e: React.ChangeEvent<HTMLInputElement>) => {
      setArrayInputVal(e.target.value);
  };

  const handleArrayBlur = () => {
      const parts = arrayInputVal.split(',').map(s => s.trim()).filter(s => s.length > 0);
      
      // Try to convert to numbers if all parts look like numbers
      const allNumbers = parts.every(p => !isNaN(Number(p)));
      if (allNumbers && parts.length > 0) {
          onChange(parts.map(Number));
      } else {
          onChange(parts);
      }
  };

  const renderControl = () => {
    switch (type) {
      case 'toggle':
        return (
          <div className="settings-toggle-container">
            <span className="settings-field-description" style={{ marginBottom: 0 }}>
              {description}
            </span>
            <input
              type="checkbox"
              className="settings-toggle"
              checked={!!value}
              onChange={handleToggleChange}
            />
          </div>
        );

      case 'slider':
        return (
          <div className="settings-slider-container">
            <input
              type="range"
              className="settings-slider"
              min={min}
              max={max}
              step={step}
              value={typeof value === 'number' ? value : min || 0}
              onChange={handleSliderChange}
            />
            <span className="settings-slider-value">
              {displayTransform ? displayTransform(value) : value}
              {unit && <span style={{ marginLeft: 2, opacity: 0.7 }}>{unit}</span>}
            </span>
          </div>
        );

      case 'dropdown':
        return (
          <Dropdown
            options={options || []}
            value={value}
            onChange={(val) => onChange(val)}
            width="100%"
          />
        );

      case 'readonly':
        let displayVal = value;
        if (typeof value === 'object') {
            displayVal = JSON.stringify(value, null, 2);
        }
        return (
          <div className="settings-readonly">
            {displayVal?.toString() || ''}
          </div>
        );

      case 'array-input':
          return (
            <div className="settings-array-container">
                <input
                    type="text"
                    className="settings-input"
                    value={arrayInputVal}
                    onChange={handleArrayChange}
                    onBlur={handleArrayBlur}
                    placeholder="Value 1, Value 2..."
                />
                <div className="settings-array-helper">Comma-separated values</div>
            </div>
          );

      case 'input':
      default:
        return (
          <input
            type={typeof value === 'number' ? "number" : "text"}
            className="settings-input"
            value={value ?? ''}
            onChange={handleInputChange}
            min={min}
            max={max}
            step={step}
          />
        );
    }
  };

  return (
    <div className="settings-field">
      <div className="settings-field-header">
        <label className="settings-field-label">
          {label}
        </label>
        {restartRequired && <RestartBadge subsystem={subsystem} />}
      </div>
      
      {/* For non-toggles, description goes here. Toggles have it inline. */}
      {type !== 'toggle' && description && (
        <div className="settings-field-description">{description}</div>
      )}

      <div className="settings-control-container">
        {renderControl()}
      </div>
      
      {error && <div className="settings-field-error" style={{ color: 'var(--status-error)', fontSize: '12px', marginTop: '4px' }}>{error}</div>}
    </div>
  );
}
