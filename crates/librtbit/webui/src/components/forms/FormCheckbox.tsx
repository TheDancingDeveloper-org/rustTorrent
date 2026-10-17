import { ChangeEventHandler } from "react";
import { settingLabel } from "../config/settingsRegistry";

export const FormCheckbox: React.FC<{
  checked: boolean;
  // Registry id of the setting: supplies the label and marks the field for settings search
  setting?: string;
  label?: string;
  name: string;
  help?: string;
  disabled?: boolean;
  inputType?: "checkbox" | "switch";
  onChange?: ChangeEventHandler<HTMLInputElement>;
  children?: React.ReactNode;
  classNames?: string;
  labelLink?: string | null;
}> = ({
  checked,
  name,
  disabled,
  onChange,
  setting,
  label,
  help,
  inputType,
  children,
  labelLink,
}) => {
  const text = setting ? settingLabel(setting) : label;
  return (
    <div className={`flex gap-3 items-start`} data-setting-id={setting}>
      <div className="flex">
        <input
          type={inputType || "checkbox"}
          className="block mt-1"
          id={name}
          name={name}
          disabled={disabled}
          checked={checked}
          onChange={onChange}
        />
      </div>
      <div className="flex flex-col gap-1">
        {labelLink ? (
          <a
            href={labelLink}
            target="_blank"
            className="text-primary hover:underline"
          >
            {text}
          </a>
        ) : (
          <label htmlFor={name}>{text}</label>
        )}

        {help && <div className="text-sm text-secondary mb-3">{help}</div>}
      </div>
      {children}
    </div>
  );
};
