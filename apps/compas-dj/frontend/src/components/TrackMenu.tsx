import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";

export interface TrackMenuItem {
  id: string;
  label: string;
  detail?: string;
  danger?: boolean;
  onSelect: () => void;
}

export function TrackMenu({
  x,
  y,
  groups,
  onClose,
}: {
  x: number;
  y: number;
  groups: TrackMenuItem[][];
  onClose: () => void;
}) {
  const menuRef = useRef<HTMLDivElement>(null);
  const [position, setPosition] = useState({ x, y });
  useLayoutEffect(() => {
    const menu = menuRef.current;
    if (!menu) return;
    const bounds = menu.getBoundingClientRect();
    const nextX = x + bounds.width > window.innerWidth ? Math.max(8, window.innerWidth - bounds.width - 8) : x;
    const nextY = y + bounds.height > window.innerHeight ? Math.max(8, window.innerHeight - bounds.height - 8) : y;
    setPosition({ x: nextX, y: nextY });
    menu.querySelector<HTMLButtonElement>("button")?.focus();
  }, [x, y]);
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        onClose();
        return;
      }
      const menu = menuRef.current;
      if (!menu || (event.key !== "ArrowDown" && event.key !== "ArrowUp")) return;
      const items = [...menu.querySelectorAll<HTMLButtonElement>("button")];
      if (items.length === 0) return;
      event.preventDefault();
      const current = items.indexOf(document.activeElement as HTMLButtonElement);
      const step = event.key === "ArrowDown" ? 1 : -1;
      const next = current < 0 ? 0 : (current + step + items.length) % items.length;
      items[next]?.focus();
    };
    const onPointer = (event: PointerEvent) => {
      if (menuRef.current && !menuRef.current.contains(event.target as Node)) onClose();
    };
    window.addEventListener("keydown", onKey);
    window.addEventListener("pointerdown", onPointer);
    window.addEventListener("scroll", onClose, true);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("pointerdown", onPointer);
      window.removeEventListener("scroll", onClose, true);
    };
  }, [onClose]);
  return createPortal(
    <div
      ref={menuRef}
      className="track-menu"
      role="menu"
      style={{ left: position.x, top: position.y }}
      onContextMenu={(event) => event.preventDefault()}
    >
      {groups.map((group, groupIndex) => (
        <div className="track-menu-group" key={group[0]?.id ?? groupIndex} role="group">
          {groupIndex > 0 && <div className="track-menu-sep" />}
          {group.map((item) => (
            <button
              key={item.id}
              type="button"
              role="menuitem"
              className={item.danger ? "track-menu-danger" : undefined}
              onClick={() => {
                item.onSelect();
                onClose();
              }}
            >
              <span>{item.label}</span>
              {item.detail && <span className="track-menu-detail">{item.detail}</span>}
            </button>
          ))}
        </div>
      ))}
    </div>,
    document.body,
  );
}
