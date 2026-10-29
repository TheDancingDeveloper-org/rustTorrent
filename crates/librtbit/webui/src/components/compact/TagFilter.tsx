import { useContext, useEffect, useMemo, useRef, useState } from "react";
import { BsTag, BsTagFill, BsPlus, BsTrash, BsPencil } from "react-icons/bs";
import { APIContext } from "../../context";
import { useUIStore } from "../../stores/uiStore";
import { useTorrentStore } from "../../stores/torrentStore";

export const TagFilter: React.FC = () => {
  const API = useContext(APIContext);
  const torrents = useTorrentStore((state) => state.torrents);
  const tagFilter = useUIStore((state) => state.tagFilter);
  const setTagFilter = useUIStore((state) => state.setTagFilter);

  const [registry, setRegistry] = useState<string[]>([]);
  const [showNewInput, setShowNewInput] = useState(false);
  const [newTagName, setNewTagName] = useState("");
  const [editing, setEditing] = useState<string | null>(null);
  const [editName, setEditName] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);
  const editRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    let cancelled = false;
    API.getTags()
      .then((tags) => {
        if (!cancelled) setRegistry(tags);
      })
      .catch(() => {
        // The tag endpoint may not be reachable yet.
      });
    return () => {
      cancelled = true;
    };
  }, [API]);

  useEffect(() => {
    if (showNewInput && inputRef.current) inputRef.current.focus();
  }, [showNewInput]);

  useEffect(() => {
    if (editing && editRef.current) editRef.current.focus();
  }, [editing]);

  const counts = useMemo(() => {
    const perTag: Record<string, number> = {};
    let untagged = 0;
    let total = 0;
    if (torrents) {
      for (const t of torrents) {
        total++;
        const tags = t.tags ?? [];
        if (tags.length === 0) untagged++;
        for (const tag of tags) perTag[tag] = (perTag[tag] ?? 0) + 1;
      }
    }
    return { perTag, untagged, total };
  }, [torrents]);

  const names = useMemo(() => {
    const all = new Set<string>(registry);
    for (const name of Object.keys(counts.perTag)) all.add(name);
    return Array.from(all).sort((a, b) => a.localeCompare(b));
  }, [registry, counts.perTag]);

  const handleCreate = async () => {
    const name = newTagName.trim();
    if (!name) {
      setShowNewInput(false);
      return;
    }
    try {
      await API.createTags([name]);
      setRegistry(await API.getTags());
    } catch {
      // A duplicate name is not an error worth surfacing here.
    }
    setNewTagName("");
    setShowNewInput(false);
  };

  const handleRename = async (from: string) => {
    const to = editName.trim();
    setEditing(null);
    if (!to || to === from) return;
    try {
      await API.renameTag(from, to);
      setRegistry(await API.getTags());
      if (tagFilter === from) setTagFilter(to);
    } catch {
      // A name already in use stays as it was.
    }
  };

  const handleDelete = async (name: string) => {
    try {
      await API.deleteTags([name]);
      setRegistry((prev) => prev.filter((tag) => tag !== name));
      if (tagFilter === name) setTagFilter(null);
    } catch {
      // Leave the row in place if the delete did not land.
    }
  };

  const activeItemClass = "bg-primary/10 text-primary font-medium";
  const inactiveItemClass =
    "text-secondary hover:bg-surface-sunken hover:text-primary";
  const iconClass = "w-3.5 h-3.5 shrink-0";

  const row = (
    key: string,
    label: string,
    count: number,
    active: boolean,
    onClick: () => void,
    icon: React.ReactNode,
    onDelete?: () => void,
  ) => (
    <div key={key} className="group flex items-center">
      <button
        onClick={onClick}
        aria-pressed={active}
        aria-label={`${label}, ${count} ${count === 1 ? "torrent" : "torrents"}`}
        className={`flex-1 min-w-0 flex items-center gap-2.5 px-2.5 py-1.5 rounded text-sm cursor-pointer transition-colors focus-ring ${
          active ? activeItemClass : inactiveItemClass
        }`}
      >
        {icon}
        <span className="flex-1 text-left truncate">{label}</span>
        <span
          className={`text-xs tabular-nums ${active ? "text-primary" : "text-tertiary"}`}
        >
          {count}
        </span>
      </button>
      {onDelete && (
        <button
          type="button"
          onClick={() => {
            setEditing(label);
            setEditName(label);
          }}
          aria-label={`Rename tag ${label}`}
          title={`Rename tag ${label}`}
          className="hidden group-hover:flex items-center justify-center w-6 h-6 text-tertiary hover:text-primary cursor-pointer focus-ring rounded"
        >
          <BsPencil className="w-3 h-3" />
        </button>
      )}
      {onDelete && (
        <button
          type="button"
          onClick={onDelete}
          aria-label={`Delete tag ${label}`}
          title={`Delete tag ${label}`}
          className="hidden group-hover:flex items-center justify-center w-6 h-6 text-tertiary hover:text-error cursor-pointer focus-ring rounded"
        >
          <BsTrash className="w-3 h-3" />
        </button>
      )}
    </div>
  );

  return (
    <div>
      <div className="px-3 pt-3 pb-1 flex items-center justify-between">
        <h2 className="text-xs font-semibold text-tertiary uppercase tracking-wider">
          Tags
        </h2>
        <button
          onClick={() => setShowNewInput(true)}
          type="button"
          className="text-tertiary hover:text-primary cursor-pointer focus-ring rounded"
          aria-label="New tag"
          title="New tag"
        >
          <BsPlus className="w-4 h-4" />
        </button>
      </div>
      <div className="px-1.5">
        {showNewInput && (
          <div className="px-2.5 py-1">
            <input
              ref={inputRef}
              type="text"
              value={newTagName}
              onChange={(e) => setNewTagName(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") handleCreate();
                if (e.key === "Escape") {
                  setNewTagName("");
                  setShowNewInput(false);
                }
              }}
              onBlur={handleCreate}
              placeholder="Tag name..."
              className="w-full px-2 py-1 text-sm bg-surface border border-divider rounded focus-ring focus:border-primary placeholder:text-tertiary"
            />
          </div>
        )}
        {row(
          "all",
          "All",
          counts.total,
          tagFilter === null,
          () => setTagFilter(null),
          <BsTag className={iconClass} />,
        )}
        {row(
          "untagged",
          "Untagged",
          counts.untagged,
          tagFilter === "",
          () => setTagFilter(""),
          <BsTag className={iconClass} />,
        )}
        {names.map((name) =>
          editing === name ? (
            <div key={name} className="px-2.5 py-1">
              <input
                ref={editRef}
                type="text"
                value={editName}
                aria-label={`Rename tag ${name}`}
                onChange={(e) => setEditName(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") handleRename(name);
                  if (e.key === "Escape") setEditing(null);
                }}
                onBlur={() => handleRename(name)}
                className="w-full px-2 py-1 text-sm bg-surface border border-divider rounded focus-ring focus:border-primary"
              />
            </div>
          ) : (
            row(
              name,
              name,
              counts.perTag[name] ?? 0,
              tagFilter === name,
              () => setTagFilter(name),
              <BsTagFill className={iconClass} />,
              () => handleDelete(name),
            )
          ),
        )}
      </div>
    </div>
  );
};
