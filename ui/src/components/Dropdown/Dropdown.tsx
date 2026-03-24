import { useState, useRef, useEffect, useCallback } from "react";
import { createPortal } from "react-dom";
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
  const [coords, setCoords] = useState<{ top: number; left: number; width: number } | null>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
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
      const target = e.target as Node;
      // Check if click is inside container (trigger) OR inside the portal list
      const inContainer = containerRef.current && containerRef.current.contains(target);
      const inList = listRef.current && listRef.current.contains(target);

      if (!inContainer && !inList) {
        setIsOpen(false);
        setSearch("");
      }
    };
    
    document.addEventListener("mousedown", handleClickOutside);
    return () => document.removeEventListener("mousedown", handleClickOutside);
  }, [isOpen]);

  // Close on scroll/resize to avoid detached dropdowns
  useEffect(() => {
    if (!isOpen) return;

    const handleScroll = (e: Event) => {
      // Ignore scrolls inside the dropdown list itself (e.g. scrolling through options)
      if (listRef.current && listRef.current.contains(e.target as Node)) return;
      setIsOpen(false);
      setSearch("");
    };

    const handleResize = () => {
      setIsOpen(false);
      setSearch("");
    };

    window.addEventListener("scroll", handleScroll, { capture: true });
    window.addEventListener("resize", handleResize);

    return () => {
      window.removeEventListener("scroll", handleScroll, { capture: true });
      window.removeEventListener("resize", handleResize);
    };
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
      // Small timeout to allow render in portal
      setTimeout(() => searchInputRef.current?.focus(), 10);
    }
  }, [isOpen, searchable]);

  const handleSelect = useCallback((optionValue: string) => {
    onChange(optionValue);
    setIsOpen(false);
    setSearch("");
  }, [onChange]);

  const toggleOpen = useCallback(() => {
    if (disabled) return;
    
    if (isOpen) {
      setIsOpen(false);
      setSearch("");
    } else {
      if (triggerRef.current) {
        const rect = triggerRef.current.getBoundingClientRect();
        setCoords({
          top: rect.bottom + 4,
          left: rect.left,
          width: rect.width
        });
        setIsOpen(true);
      }
    }
  }, [disabled, isOpen]);

  return (
    <div 
      ref={containerRef} 
      className={`dropdown ${disabled ? "dropdown--disabled" : ""}`}
      style={width ? { width } : undefined}
    >
      <button
        ref={triggerRef}
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
      
      {isOpen && coords && createPortal(
        <div 
          ref={listRef}
          className="dropdown__list"
          style={{
            position: "fixed",
            top: coords.top,
            left: coords.left,
            width: coords.width,
          }}
        >
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
        </div>,
        document.body
      )}
    </div>
  );
}
