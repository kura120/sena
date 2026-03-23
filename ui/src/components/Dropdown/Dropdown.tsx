import { useState, useRef, useEffect, useCallback } from "react";
import { IconChevronDown, IconCheck } from "../icons";
import "./Dropdown.css";

interface DropdownOption {
  value: string;
  label: string;
  description?: string;
}

interface DropdownProps {
  options: DropdownOption[];
  value: string;
  onChange: (value: string) => void;
  placeholder?: string;
  searchable?: boolean;
  disabled?: boolean;
  width?: string;
}

export function Dropdown({ options, value, onChange, placeholder, searchable, disabled, width }: DropdownProps) {
  const [isOpen, setIsOpen] = useState(false);
  const [search, setSearch] = useState("");
  const containerRef = useRef<HTMLDivElement>(null);
  const searchInputRef = useRef<HTMLInputElement>(null);

  const selectedOption = options.find(o => o.value === value);
  
  const filteredOptions = searchable && search
    ? options.filter(o => 
        o.label.toLowerCase().includes(search.toLowerCase()) ||
        (o.description && o.description.toLowerCase().includes(search.toLowerCase()))
      )
    : options;

  // Close on outside click
  useEffect(() => {
    if (!isOpen) return;
    
    const handleClickOutside = (e: MouseEvent) => {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
        setIsOpen(false);
        setSearch("");
      }
    };
    
    document.addEventListener("mousedown", handleClickOutside);
    return () => document.removeEventListener("mousedown", handleClickOutside);
  }, [isOpen]);

  // Close on Escape
  useEffect(() => {
    if (!isOpen) return;
    
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        setIsOpen(false);
        setSearch("");
      }
    };
    
    document.addEventListener("keydown", handleKeyDown);
    return () => document.removeEventListener("keydown", handleKeyDown);
  }, [isOpen]);

  // Focus search input when opened
  useEffect(() => {
    if (isOpen && searchable && searchInputRef.current) {
      searchInputRef.current.focus();
    }
  }, [isOpen, searchable]);

  const handleSelect = useCallback((optionValue: string) => {
    onChange(optionValue);
    setIsOpen(false);
    setSearch("");
  }, [onChange]);

  const toggleOpen = useCallback(() => {
    if (!disabled) {
      setIsOpen(prev => !prev);
      if (isOpen) setSearch("");
    }
  }, [disabled, isOpen]);

  return (
    <div 
      ref={containerRef} 
      className={`dropdown ${disabled ? "dropdown--disabled" : ""}`}
      style={width ? { width } : undefined}
    >
      <button
        type="button"
        className={`dropdown__trigger ${isOpen ? "dropdown__trigger--open" : ""}`}
        onClick={toggleOpen}
        disabled={disabled}
      >
        <span className="dropdown__trigger-text">
          {selectedOption ? selectedOption.label : (placeholder || "Select...")}
        </span>
        <IconChevronDown size={14} className="dropdown__trigger-icon" />
      </button>
      
      {isOpen && (
        <div className="dropdown__list">
          {searchable && (
            <input
              ref={searchInputRef}
              type="text"
              className="dropdown__search"
              placeholder="Search..."
              value={search}
              onChange={(e) => setSearch(e.target.value)}
            />
          )}
          <div className="dropdown__options">
            {filteredOptions.length > 0 ? (
              filteredOptions.map(option => (
                <button
                  key={option.value}
                  type="button"
                  className={`dropdown__option ${option.value === value ? "dropdown__option--selected" : ""}`}
                  onClick={() => handleSelect(option.value)}
                >
                  <div className="dropdown__option-content">
                    <span className="dropdown__option-label">{option.label}</span>
                    {option.description && (
                      <span className="dropdown__option-desc">{option.description}</span>
                    )}
                  </div>
                  {option.value === value && <IconCheck size={14} className="dropdown__option-check" />}
                </button>
              ))
            ) : (
                <div className="dropdown__empty">No matches</div>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
