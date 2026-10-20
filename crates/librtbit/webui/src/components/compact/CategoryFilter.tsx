import { useContext, useEffect, useMemo, useRef, useState } from "react";
import { BsTag, BsTagFill, BsPlus } from "react-icons/bs";
import { APIContext } from "../../context";
import { useUIStore } from "../../stores/uiStore";
import { useTorrentStore } from "../../stores/torrentStore";

export const CategoryFilter: React.FC = () => {
  const API = useContext(APIContext);
  const torrents = useTorrentStore((state) => state.torrents);
  const categoryFilter = useUIStore((state) => state.categoryFilter);
  const setCategoryFilter = useUIStore((state) => state.setCategoryFilter);
  const categories = useUIStore((state) => state.categories);
  const setCategories = useUIStore((state) => state.setCategories);

  const [showNewInput, setShowNewInput] = useState(false);
  const [newCategoryName, setNewCategoryName] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);

  // Fetch categories on mount
  useEffect(() => {
    let cancelled = false;
    API.getCategories()
      .then((cats) => {
        if (!cancelled) setCategories(cats);
      })
      .catch(() => {
        // Categories endpoint may not exist yet on the backend
      });
    return () => {
      cancelled = true;
    };
  }, [API, setCategories]);

  // Focus input when it appears
  useEffect(() => {
    if (showNewInput && inputRef.current) {
      inputRef.current.focus();
    }
  }, [showNewInput]);

  // Count torrents per category
  const categoryCounts = useMemo(() => {
    const counts: Record<string, number> = {};
    let uncategorized = 0;
    let total = 0;

    if (torrents) {
      for (const t of torrents) {
        total++;
        if (t.category) {
          counts[t.category] = (counts[t.category] || 0) + 1;
        } else {
          uncategorized++;
        }
      }
    }

    return { counts, uncategorized, total };
  }, [torrents]);

  // Build list of category names (from both API categories and torrent data)
  const categoryNames = useMemo(() => {
    const names = new Set<string>();
    for (const name of Object.keys(categories)) {
      names.add(name);
    }
    for (const name of Object.keys(categoryCounts.counts)) {
      names.add(name);
    }
    return Array.from(names).sort((a, b) => a.localeCompare(b));
  }, [categories, categoryCounts.counts]);

  const handleCreateCategory = async () => {
    const name = newCategoryName.trim();
    if (!name) {
      setShowNewInput(false);
      return;
    }
    try {
      await API.createCategory(name);
      const cats = await API.getCategories();
      setCategories(cats);
    } catch {
      // ignore - category may already exist
    }
    setNewCategoryName("");
    setShowNewInput(false);
  };

  const activeItemClass = "bg-primary/10 text-primary font-medium";
  const inactiveItemClass =
    "text-secondary hover:bg-surface-sunken hover:text-primary";
  const iconClass = "w-3.5 h-3.5 shrink-0";

  return (
    <div>
      <div className="px-3 pt-3 pb-1 flex items-center justify-between">
        <h2 className="text-xs font-semibold text-tertiary uppercase tracking-wider">
          Categories
        </h2>
        <button
          onClick={() => setShowNewInput(true)}
          type="button"
          className="text-tertiary hover:text-primary cursor-pointer focus-ring rounded"
          aria-label="New category"
          title="New category"
        >
          <BsPlus className="w-4 h-4" />
        </button>
      </div>
      <div className="px-1.5">
        {/* New category input */}
        {showNewInput && (
          <div className="px-2.5 py-1">
            <input
              ref={inputRef}
              type="text"
              value={newCategoryName}
              onChange={(e) => setNewCategoryName(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") handleCreateCategory();
                if (e.key === "Escape") {
                  setNewCategoryName("");
                  setShowNewInput(false);
                }
              }}
              onBlur={handleCreateCategory}
              placeholder="Category name..."
              className="w-full px-2 py-1 text-sm bg-surface border border-divider rounded focus-ring focus:border-primary placeholder:text-tertiary"
            />
          </div>
        )}

        {/* All */}
        <button
          onClick={() => setCategoryFilter(null)}
          aria-pressed={categoryFilter === null}
          aria-label={`All, ${categoryCounts.total} ${categoryCounts.total === 1 ? "torrent" : "torrents"}`}
          className={`w-full flex items-center gap-2.5 px-2.5 py-1.5 rounded text-sm cursor-pointer transition-colors focus-ring ${
            categoryFilter === null ? activeItemClass : inactiveItemClass
          }`}
        >
          <BsTag className={iconClass} />
          <span className="flex-1 text-left">All</span>
          <span
            className={`text-xs tabular-nums ${
              categoryFilter === null ? "text-primary" : "text-tertiary"
            }`}
          >
            {categoryCounts.total}
          </span>
        </button>

        {/* Uncategorized */}
        <button
          onClick={() => setCategoryFilter("")}
          aria-pressed={categoryFilter === ""}
          aria-label={`Uncategorized, ${categoryCounts.uncategorized} ${categoryCounts.uncategorized === 1 ? "torrent" : "torrents"}`}
          className={`w-full flex items-center gap-2.5 px-2.5 py-1.5 rounded text-sm cursor-pointer transition-colors focus-ring ${
            categoryFilter === "" ? activeItemClass : inactiveItemClass
          }`}
        >
          <BsTag className={iconClass} />
          <span className="flex-1 text-left">Uncategorized</span>
          <span
            className={`text-xs tabular-nums ${
              categoryFilter === "" ? "text-primary" : "text-tertiary"
            }`}
          >
            {categoryCounts.uncategorized}
          </span>
        </button>

        {/* Each category */}
        {categoryNames.map((name) => {
          const count = categoryCounts.counts[name] ?? 0;
          return (
            <button
              key={name}
              onClick={() => setCategoryFilter(name)}
              aria-pressed={categoryFilter === name}
              aria-label={`${name}, ${count} ${count === 1 ? "torrent" : "torrents"}`}
              className={`w-full flex items-center gap-2.5 px-2.5 py-1.5 rounded text-sm cursor-pointer transition-colors focus-ring ${
                categoryFilter === name ? activeItemClass : inactiveItemClass
              }`}
            >
              <BsTagFill className={iconClass} />
              <span className="flex-1 text-left truncate">{name}</span>
              <span
                className={`text-xs tabular-nums ${
                  categoryFilter === name ? "text-primary" : "text-tertiary"
                }`}
              >
                {count}
              </span>
            </button>
          );
        })}
      </div>
    </div>
  );
};
