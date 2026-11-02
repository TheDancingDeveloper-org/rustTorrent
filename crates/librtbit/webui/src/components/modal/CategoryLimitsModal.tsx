import { useContext, useState } from "react";
import { APIContext } from "../../context";
import { CategoryInfo } from "../../api-types";
import {
  limitFieldsFromCategory,
  parseCategoryLimits,
} from "../../helper/categoryLimits";
import { Button } from "../buttons/Button";
import { Modal } from "./Modal";
import { ModalBody } from "./ModalBody";
import { ModalFooter } from "./ModalFooter";

const inputClass =
  "w-full px-2 py-1 text-sm bg-surface border border-divider rounded focus-ring focus:border-primary placeholder:text-tertiary";

export const CategoryLimitsModal: React.FC<{
  category: CategoryInfo;
  onClose: () => void;
  onSaved: () => void;
}> = ({ category, onClose, onSaved }) => {
  const API = useContext(APIContext);
  const [fields, setFields] = useState(() => limitFieldsFromCategory(category));
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  const save = async () => {
    const parsed = parseCategoryLimits(fields);
    if ("error" in parsed) {
      setError(parsed.error);
      return;
    }
    setSaving(true);
    try {
      // Editing replaces the category, so keep its save path.
      await API.createCategory(
        category.name,
        category.save_path,
        parsed.values,
      );
      onSaved();
    } catch (e: any) {
      setError(e?.text ?? e?.message ?? "Could not save the category.");
      setSaving(false);
    }
  };

  const field = (
    label: string,
    key: "ratio" | "seedingMinutes" | "inactiveMinutes",
    inputMode: "decimal" | "numeric",
  ) => (
    <label className="block mb-3 text-sm">
      <span className="block mb-1">{label}</span>
      <input
        type="text"
        inputMode={inputMode}
        value={fields[key]}
        placeholder="Use global limit"
        onChange={(e) => setFields({ ...fields, [key]: e.target.value })}
        className={inputClass}
      />
    </label>
  );

  return (
    <Modal
      isOpen={true}
      onClose={onClose}
      title={`Share limits: ${category.name}`}
    >
      <ModalBody>
        {field("Ratio limit", "ratio", "decimal")}
        {field("Seeding time limit (minutes)", "seedingMinutes", "numeric")}
        {field(
          "Inactive seeding time limit (minutes)",
          "inactiveMinutes",
          "numeric",
        )}
        {error && (
          <div role="alert" className="text-sm text-error">
            {error}
          </div>
        )}
      </ModalBody>
      <ModalFooter>
        <Button variant="cancel" onClick={onClose}>
          Cancel
        </Button>
        <Button variant="primary" onClick={save} disabled={saving}>
          Save
        </Button>
      </ModalFooter>
    </Modal>
  );
};
