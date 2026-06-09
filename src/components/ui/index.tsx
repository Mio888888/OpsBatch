import { Children, cloneElement, createContext, isValidElement, useContext, useEffect, useMemo, useRef, useState } from 'react';
import type {
  CSSProperties,
  ChangeEvent,
  HTMLAttributes,
  InputHTMLAttributes,
  Key,
  KeyboardEvent,
  MouseEvent,
  ReactElement,
  ReactNode,
  TextareaHTMLAttributes,
} from 'react';
import * as DialogPrimitive from '@radix-ui/react-dialog';
import * as AlertDialogPrimitive from '@radix-ui/react-alert-dialog';
import * as RadixSelect from '@radix-ui/react-select';
import * as RadixSwitch from '@radix-ui/react-switch';
import * as RadixTabs from '@radix-ui/react-tabs';
import * as RadixToast from '@radix-ui/react-toast';
import * as RadixTooltip from '@radix-ui/react-tooltip';
import Treeselect from 'treeselectjs';
import type { OptionType, ValueType } from 'treeselectjs';
import { translateText, useTranslation } from '../../i18n';
import 'treeselectjs/dist/treeselectjs.css';

export type BadgeStatus = 'success' | 'processing' | 'default' | 'error' | 'warning';
export interface BadgeProps {
  status?: BadgeStatus;
  text?: ReactNode;
}

export type ColumnsType<T> = Array<ColumnType<T>>;

export interface ColumnType<T> {
  title?: ReactNode;
  dataIndex?: keyof T | string;
  key?: string;
  width?: number | string;
  ellipsis?: boolean;
  render?: (value: never, record: T, index: number) => ReactNode;
  sorter?: (a: T, b: T) => number;
  filters?: { text: ReactNode; value: string | number | boolean }[];
  onFilter?: (value: never, record: T) => boolean;
}

type Size = 'small' | 'middle' | 'large';
type ButtonType = 'primary' | 'link' | 'text' | 'default';
type FieldName = string | number | symbol;

function cx(...values: Array<string | false | null | undefined>) {
  return values.filter(Boolean).join(' ');
}

function readValue<T>(record: T, dataIndex?: keyof T | string) {
  if (!dataIndex) return undefined;
  return (record as Record<string, unknown>)[String(dataIndex)];
}

function getRowKey<T>(rowKey: keyof T | string | ((record: T) => string), record: T, index: number) {
  if (typeof rowKey === 'function') return rowKey(record);
  const value = (record as Record<string, unknown>)[String(rowKey)];
  return value == null ? String(index) : String(value);
}

function isEmptyValue(value: unknown) {
  return value == null || value === '' || (Array.isArray(value) && value.length === 0);
}

function normalizeOptions<T extends string | number>(options?: SelectOption<T>[]) {
  return options ?? [];
}

export function Button({
  children,
  type = 'default',
  size = 'middle',
  danger,
  icon,
  loading,
  className,
  disabled,
  ...props
}: {
  children?: ReactNode;
  type?: ButtonType;
  size?: Size;
  danger?: boolean;
  icon?: ReactNode;
  loading?: boolean;
  htmlType?: 'button' | 'submit' | 'reset';
} & Omit<React.ButtonHTMLAttributes<HTMLButtonElement>, 'type'>) {
  return (
    <button
      {...props}
      type={props.htmlType ?? 'button'}
      disabled={disabled || loading}
      className={cx('ui-button', `ui-button-${type}`, `ui-button-${size}`, danger && 'ui-button-danger', className)}
    >
      {loading ? <span className="ui-spinner ui-spinner-inline" /> : icon ? <span className="ui-button-icon">{icon}</span> : null}
      {children ? <span>{children}</span> : null}
    </button>
  );
}

export function Space({
  children,
  direction = 'horizontal',
  size = 8,
  wrap,
  className,
  style,
  ...props
}: {
  children?: ReactNode;
  direction?: 'horizontal' | 'vertical';
  size?: number | Size;
  wrap?: boolean;
} & HTMLAttributes<HTMLDivElement>) {
  const gap = typeof size === 'number' ? size : size === 'small' ? 4 : size === 'large' ? 16 : 8;
  return (
    <div
      {...props}
      className={cx('ui-space', direction === 'vertical' && 'ui-space-vertical', wrap && 'ui-space-wrap', className)}
      style={{ gap, ...style }}
    >
      {children}
    </div>
  );
}

Space.Compact = function SpaceCompact({ children, className, ...props }: HTMLAttributes<HTMLDivElement>) {
  return <div {...props} className={cx('ui-space-compact', className)}>{children}</div>;
};

function BaseInput(props: Omit<InputHTMLAttributes<HTMLInputElement>, 'size' | 'prefix'> & { allowClear?: boolean; prefix?: ReactNode; size?: Size }) {
  const { className, allowClear: _allowClear, prefix, size: inputSize, ...rest } = props;
  const input = <input {...rest} className={cx('ui-input', inputSize && `ui-input-${inputSize}`, className)} />;
  if (!prefix) return input;
  return <span className="ui-input-prefix">{prefix}{input}</span>;
}

function TextArea({ className, ...props }: TextareaHTMLAttributes<HTMLTextAreaElement> & { autoSize?: boolean | { minRows?: number; maxRows?: number } }) {
  return <textarea {...props} className={cx('ui-textarea', className)} />;
}

function Password(props: Omit<InputHTMLAttributes<HTMLInputElement>, 'size' | 'prefix'> & { allowClear?: boolean; prefix?: ReactNode; size?: Size }) {
  return <BaseInput {...props} type="password" />;
}

function Search(props: Omit<InputHTMLAttributes<HTMLInputElement>, 'size' | 'prefix'> & { allowClear?: boolean; prefix?: ReactNode; size?: Size }) {
  return <BaseInput {...props} type="search" />;
}

export const Input = Object.assign(BaseInput, { TextArea, Password, Search });

export function InputNumber({
  value,
  onChange,
  addonAfter,
  className,
  style,
  ...props
}: Omit<InputHTMLAttributes<HTMLInputElement>, 'value' | 'onChange' | 'type'> & {
  value?: number;
  onChange?: (value: number | null) => void;
  addonAfter?: ReactNode;
}) {
  const input = (
    <input
      {...props}
      type="number"
      value={value ?? ''}
      onChange={(event) => {
        const next = event.target.value;
        onChange?.(next === '' ? null : Number(next));
      }}
      className={cx('ui-input', className)}
      style={addonAfter ? undefined : style}
    />
  );

  if (!addonAfter) return input;
  return (
    <span className="ui-input-addon" style={style}>
      {input}
      <span className="ui-input-addon-after">{addonAfter}</span>
    </span>
  );
}

export interface SelectOption<T extends string | number = string | number> {
  value: T;
  label: ReactNode;
}

type SelectValue<T extends string | number = string | number> = T | T[] | null | undefined;

type SingleSelectProps<T extends string | number> = {
  value?: T | null;
  defaultValue?: T | null;
  onChange?: (value: T) => void;
  options?: SelectOption<T>[];
  placeholder?: ReactNode;
  allowClear?: boolean;
  mode?: undefined;
  disabled?: boolean;
  className?: string;
  style?: CSSProperties;
  size?: Size;
};

type MultiSelectProps<T extends string | number> = {
  value?: T[];
  defaultValue?: T[];
  onChange?: (value: T[]) => void;
  options?: SelectOption<T>[];
  placeholder?: ReactNode;
  allowClear?: boolean;
  mode: 'multiple' | 'tags';
  disabled?: boolean;
  className?: string;
  style?: CSSProperties;
  size?: Size;
};

type SelectProps<T extends string | number> = SingleSelectProps<T> | MultiSelectProps<T>;

export function Select<T extends string | number = string>(props: SelectProps<T>) {
  const { t } = useTranslation();
  const {
    value,
    defaultValue,
    onChange,
    options,
    placeholder,
    allowClear,
    mode,
    disabled,
    className,
    style,
    size,
  } = props;
  const opts = normalizeOptions(options);
  const initial = value ?? defaultValue ?? (mode ? [] : '');
  const [inner, setInner] = useState<SelectValue<T>>(initial as SelectValue<T>);
  const [multiOpen, setMultiOpen] = useState(false);
  const multiRef = useRef<HTMLDivElement>(null);
  const current = value ?? inner;

  useEffect(() => {
    if (!mode || !multiOpen) return undefined;

    const handlePointerDown = (event: globalThis.MouseEvent) => {
      if (!multiRef.current?.contains(event.target as Node)) {
        setMultiOpen(false);
      }
    };

    document.addEventListener('mousedown', handlePointerDown);
    return () => document.removeEventListener('mousedown', handlePointerDown);
  }, [mode, multiOpen]);

  if (mode) {
    const selectedValues = Array.isArray(current) ? current : [];
    const selectedKeys = new Set(selectedValues.map(String));
    const selectedItems = selectedValues.map((selectedValue) => {
      const option = opts.find((item) => String(item.value) === String(selectedValue));
      return {
        value: selectedValue,
        label: option?.label ?? String(selectedValue),
      };
    });

    const updateSelected = (next: T[]) => {
      setInner(next);
      (onChange as ((value: T[]) => void) | undefined)?.(next);
    };

    const toggleOption = (optionValue: T) => {
      const optionKey = String(optionValue);
      const next = selectedKeys.has(optionKey)
        ? selectedValues.filter((item) => String(item) !== optionKey)
        : [...selectedValues, optionValue];
      updateSelected(next);
    };

    return (
      <div
        ref={multiRef}
        className={cx('ui-multi-select', disabled && 'ui-multi-select-disabled', size && `ui-select-${size}`, className)}
        style={style}
      >
        <button
          type="button"
          className="ui-multi-select-trigger"
          disabled={disabled}
          aria-haspopup="listbox"
          aria-expanded={multiOpen}
          onClick={() => setMultiOpen((open) => !open)}
        >
          <span className={cx('ui-multi-select-values', selectedItems.length === 0 && 'ui-multi-select-placeholder')}>
            {selectedItems.length > 0 ? selectedItems.map((item) => (
              <span key={String(item.value)} className="ui-multi-select-tag">
                <span className="ui-multi-select-tag-text">{item.label}</span>
              </span>
            )) : placeholder}
          </span>
          {allowClear && selectedItems.length > 0 ? (
            <span
              role="button"
              tabIndex={-1}
              className="ui-multi-select-clear"
              aria-label={String(t('common.clearSelection'))}
              onClick={(event) => {
                event.preventDefault();
                event.stopPropagation();
                updateSelected([]);
              }}
            >
              ×
            </span>
          ) : null}
          <span className="ui-multi-select-arrow" aria-hidden="true">▾</span>
        </button>
        {multiOpen ? (
          <div className="ui-multi-select-dropdown" role="listbox" aria-multiselectable="true">
            {opts.length > 0 ? opts.map((option) => {
              const optionKey = String(option.value);
              const checked = selectedKeys.has(optionKey);
              return (
                <button
                  key={optionKey}
                  type="button"
                  role="option"
                  aria-selected={checked}
                  className={cx('ui-multi-select-option', checked && 'ui-multi-select-option-selected')}
                  onClick={() => toggleOption(option.value)}
                >
                  <span className="ui-multi-select-checkbox" aria-hidden="true">{checked ? '✓' : ''}</span>
                  <span className="ui-multi-select-option-label">{option.label}</span>
                </button>
              );
            }) : <div className="ui-multi-select-empty">{t('common.noOptions')}</div>}
          </div>
        ) : null}
      </div>
    );
  }

  const stringValue = current == null ? '' : String(current);
  return (
    <RadixSelect.Root
      value={stringValue}
      disabled={disabled}
      onValueChange={(next) => {
        if (next === '__clear__') {
          setInner('' as T);
          (onChange as ((value: T) => void) | undefined)?.('' as T);
          return;
        }
        setInner(next as T);
        (onChange as ((value: T) => void) | undefined)?.(next as T);
      }}
    >
      <RadixSelect.Trigger className={cx('ui-select-trigger', size && `ui-select-${size}`, className)} style={style}>
        <RadixSelect.Value placeholder={placeholder} />
      </RadixSelect.Trigger>
      <RadixSelect.Portal>
        <RadixSelect.Content className="ui-select-content" position="popper">
          <RadixSelect.Viewport>
            {allowClear && <RadixSelect.Item className="ui-select-item" value="__clear__"><RadixSelect.ItemText>{t('common.clearSelection')}</RadixSelect.ItemText></RadixSelect.Item>}
            {opts.map((option) => (
              <RadixSelect.Item key={String(option.value)} className="ui-select-item" value={String(option.value)}>
                <RadixSelect.ItemText>{option.label}</RadixSelect.ItemText>
              </RadixSelect.Item>
            ))}
          </RadixSelect.Viewport>
        </RadixSelect.Content>
      </RadixSelect.Portal>
    </RadixSelect.Root>
  );
}

interface FormContextValue {
  values: Record<string, unknown>;
  setValue: (name: FieldName, value: unknown) => void;
  getValue: (name: FieldName) => unknown;
  registerRules: (name: FieldName, rules?: FormRule[]) => void;
}

interface FormRule {
  required?: boolean;
  message?: string;
}

export interface FormInstance<T extends object = Record<string, never>> {
  validateFields: () => Promise<T>;
  setFieldsValue: (values: Partial<T>) => void;
  setFieldValue: (name: keyof T | string, value: unknown) => void;
  getFieldValue: (name: keyof T | string) => unknown;
  getFieldsValue: () => T;
  resetFields: () => void;
  _subscribe: (listener: () => void) => () => void;
  _getValues: () => Record<string, unknown>;
  _setValues: (updater: (values: Record<string, unknown>) => Record<string, unknown>) => void;
  _registerRules: (name: FieldName, rules?: FormRule[]) => void;
}

const FormContext = createContext<FormContextValue | null>(null);

function createForm<T extends object = Record<string, never>>(): FormInstance<T> {
  let values: Record<string, unknown> = {};
  let initialValues: Record<string, unknown> = {};
  const rules = new Map<FieldName, FormRule[]>();
  const listeners = new Set<() => void>();

  const notify = () => listeners.forEach((listener) => listener());

  return {
    validateFields: async () => {
      for (const [name, fieldRules] of rules.entries()) {
        if (fieldRules.some((rule) => rule.required) && isEmptyValue(values[String(name)])) {
          const message = fieldRules.find((rule) => rule.required)?.message || translateText(document.documentElement.lang === 'en-US' ? 'en-US' : 'zh-CN', 'common.required');
          return Promise.reject(new Error(message));
        }
      }
      return values as T;
    },
    setFieldsValue: (next) => {
      values = { ...values, ...next };
      initialValues = { ...initialValues, ...next };
      notify();
    },
    setFieldValue: (name, value) => {
      values = { ...values, [String(name)]: value };
      if (!(String(name) in initialValues)) initialValues[String(name)] = value;
      notify();
    },
    getFieldValue: (name) => values[String(name)],
    getFieldsValue: () => values as T,
    resetFields: () => {
      values = {};
      initialValues = {};
      notify();
    },
    _subscribe: (listener) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    _getValues: () => values,
    _setValues: (updater) => {
      values = updater(values);
      notify();
    },
    _registerRules: (name, nextRules) => {
      if (nextRules) rules.set(name, nextRules);
    },
  };
}

function useForm<T extends object = Record<string, never>>(): [FormInstance<T>] {
  const ref = useRef<FormInstance<T> | null>(null);
  if (!ref.current) ref.current = createForm<T>();
  const [, setTick] = useState(0);
  useEffect(() => ref.current?._subscribe(() => setTick((tick) => tick + 1)), []);
  return [ref.current];
}

function FormRoot<T extends object = Record<string, never>>({
  children,
  form,
  layout,
  className,
  ...props
}: {
  children?: ReactNode;
  form?: FormInstance<T>;
  layout?: 'vertical' | 'horizontal';
} & HTMLAttributes<HTMLFormElement>) {
  const [internalForm] = useForm<T>();
  const instance = form ?? internalForm;
  const [, setTick] = useState(0);
  useEffect(() => instance._subscribe(() => setTick((tick) => tick + 1)), [instance]);
  const contextValue: FormContextValue = {
    values: instance._getValues(),
    setValue: (name, value) => instance.setFieldValue(String(name), value),
    getValue: (name) => instance.getFieldValue(String(name)),
    registerRules: (name, rules) => instance._registerRules(name, rules),
  };

  return (
    <FormContext.Provider value={contextValue}>
      <form {...props} className={cx('ui-form', layout === 'vertical' && 'ui-form-vertical', className)}>
        {children}
      </form>
    </FormContext.Provider>
  );
}

function bindField(child: ReactElement, name: FieldName, context: FormContextValue, valuePropName?: string) {
  const currentValue = context.getValue(name);
  const propName = valuePropName || 'value';
  const nextProps: Record<string, unknown> = {
    [propName]: propName === 'checked' ? Boolean(currentValue) : currentValue ?? '',
  };

  if (propName === 'checked') {
    nextProps.onCheckedChange = (checked: boolean) => context.setValue(name, checked);
    nextProps.onChange = (event: ChangeEvent<HTMLInputElement>) => context.setValue(name, event.target.checked);
  } else if (child.type === Select) {
    nextProps.onChange = (value: unknown) => context.setValue(name, value);
  } else {
    nextProps.onChange = (eventOrValue: ChangeEvent<HTMLInputElement | HTMLTextAreaElement> | unknown) => {
      if (eventOrValue && typeof eventOrValue === 'object' && 'target' in eventOrValue) {
        const event = eventOrValue as ChangeEvent<HTMLInputElement | HTMLTextAreaElement>;
        const target = event.target as HTMLInputElement | HTMLTextAreaElement;
        if (target instanceof HTMLInputElement && target.type === 'number') {
          context.setValue(name, target.value === '' ? undefined : Number(target.value));
        } else {
          context.setValue(name, target.value);
        }
      } else {
        context.setValue(name, eventOrValue);
      }
    };
  }

  return cloneElement(child, nextProps);
}

function FormItem({
  children,
  name,
  label,
  rules,
  noStyle,
  shouldUpdate,
  valuePropName,
  initialValue,
  extra,
  className,
  style,
}: {
  children?: ReactNode | ((form: Pick<FormInstance, 'getFieldValue' | 'setFieldValue'>) => ReactNode);
  name?: FieldName;
  label?: ReactNode;
  rules?: FormRule[];
  noStyle?: boolean;
  shouldUpdate?: boolean | ((prev: Record<string, unknown>, cur: Record<string, unknown>) => boolean);
  valuePropName?: string;
  initialValue?: unknown;
  extra?: ReactNode;
  className?: string;
  style?: CSSProperties;
}) {
  const context = useContext(FormContext);
  if (!context) return <>{children as ReactNode}</>;

  const nameRef = useRef(name);
  nameRef.current = name;
  const initialValueRef = useRef(initialValue);
  initialValueRef.current = initialValue;

  if (name) {
    context.registerRules(name, rules);
  }

  useEffect(() => {
    if (nameRef.current && initialValueRef.current !== undefined) {
      const ctx = context;
      if (ctx.getValue(nameRef.current) === undefined) {
        ctx.setValue(nameRef.current, initialValueRef.current);
      }
    }
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  if (typeof children === 'function') {
    return <>{children({ getFieldValue: (field) => context.getValue(field), setFieldValue: (field, value) => context.setValue(field, value) })}</>;
  }

  const childArray = Children.toArray(children);
  const content = name && childArray.length === 1 && isValidElement(childArray[0])
    ? bindField(childArray[0], name, context, valuePropName)
    : children;

  if (noStyle || shouldUpdate) return <>{content}</>;

  return (
    <label className={cx('ui-form-item', className)} style={style}>
      {label ? <span className="ui-form-label">{label}</span> : null}
      {content}
      {extra ? <span className="ui-form-extra">{extra}</span> : null}
    </label>
  );
}

export const Form = Object.assign(FormRoot, { Item: FormItem, useForm });

export function Modal({
  open,
  onCancel,
  onOk,
  title,
  children,
  footer,
  width,
  okText,
  cancelText,
  okButtonProps,
  cancelButtonProps,
  className,
}: {
  open?: boolean;
  onCancel?: () => void;
  onOk?: () => void;
  title?: ReactNode;
  children?: ReactNode;
  footer?: ReactNode | null;
  width?: number | string;
  okText?: ReactNode;
  cancelText?: ReactNode;
  okButtonProps?: { danger?: boolean };
  cancelButtonProps?: { danger?: boolean };
  className?: string;
  destroyOnHidden?: boolean;
  destroyOnClose?: boolean;
  maskClosable?: boolean;
  closable?: boolean;
}) {
  const { t } = useTranslation();

  return (
    <DialogPrimitive.Root open={open} onOpenChange={(next) => { if (!next) onCancel?.(); }}>
      <DialogPrimitive.Portal>
        <DialogPrimitive.Overlay className="ui-dialog-overlay" />
        <DialogPrimitive.Content className={cx('ui-dialog-content', className)} style={{ width }} aria-describedby={undefined}>
          {title ? <DialogPrimitive.Title className="ui-dialog-title">{title}</DialogPrimitive.Title> : null}
          <div className="ui-dialog-body">{children}</div>
          {footer !== null ? (
            <div className="ui-dialog-footer">
              {footer ?? (
                <>
                  <Button onClick={onCancel} danger={cancelButtonProps?.danger}>{cancelText ?? t('common.cancel')}</Button>
                  <Button type="primary" danger={okButtonProps?.danger} onClick={onOk}>{okText ?? t('common.ok')}</Button>
                </>
              )}
            </div>
          ) : null}
        </DialogPrimitive.Content>
      </DialogPrimitive.Portal>
    </DialogPrimitive.Root>
  );
}

export function Popconfirm({ children, title, onConfirm }: { children?: ReactNode; title?: ReactNode; onConfirm?: () => void }) {
  const { t } = useTranslation();

  return (
    <AlertDialogPrimitive.Root>
      <AlertDialogPrimitive.Trigger asChild>{children as ReactElement}</AlertDialogPrimitive.Trigger>
      <AlertDialogPrimitive.Portal>
        <AlertDialogPrimitive.Overlay className="ui-dialog-overlay" />
        <AlertDialogPrimitive.Content className="ui-alert-content">
          <AlertDialogPrimitive.Title className="ui-dialog-title">{title}</AlertDialogPrimitive.Title>
          <div className="ui-dialog-footer">
            <AlertDialogPrimitive.Cancel asChild><Button>{t('common.cancel')}</Button></AlertDialogPrimitive.Cancel>
            <AlertDialogPrimitive.Action asChild><Button type="primary" danger onClick={onConfirm}>{t('common.ok')}</Button></AlertDialogPrimitive.Action>
          </div>
        </AlertDialogPrimitive.Content>
      </AlertDialogPrimitive.Portal>
    </AlertDialogPrimitive.Root>
  );
}

export function Tooltip({ title, children }: { title?: ReactNode; children?: ReactNode; placement?: string }) {
  if (!title) return <>{children}</>;
  return (
    <RadixTooltip.Provider delayDuration={200}>
      <RadixTooltip.Root>
        <RadixTooltip.Trigger asChild>{children as ReactElement}</RadixTooltip.Trigger>
        <RadixTooltip.Portal>
          <RadixTooltip.Content className="ui-tooltip" sideOffset={6}>{title}<RadixTooltip.Arrow className="ui-tooltip-arrow" /></RadixTooltip.Content>
        </RadixTooltip.Portal>
      </RadixTooltip.Root>
    </RadixTooltip.Provider>
  );
}

export function Tabs({
  activeKey,
  defaultActiveKey,
  onChange,
  items,
  onEdit,
  style,
  className,
  destroyInactiveTabPane = false,
  tabBarExtraContent,
  onTabContextMenu,
  onTabListContextMenu,
}: {
  activeKey?: string;
  defaultActiveKey?: string;
  onChange?: (key: string) => void;
  items?: { key: string; label: ReactNode; children: ReactNode | ((active: boolean) => ReactNode); closable?: boolean; closeLabel?: string }[];
  type?: string;
  hideAdd?: boolean;
  style?: CSSProperties;
  className?: string;
  destroyInactiveTabPane?: boolean;
  tabBarExtraContent?: ReactNode;
  onEdit?: (targetKey: string | MouseEvent | KeyboardEvent, action: 'add' | 'remove') => void;
  onTabContextMenu?: (event: MouseEvent<HTMLDivElement>, key: string) => void;
  onTabListContextMenu?: (event: MouseEvent<HTMLDivElement>) => void;
}) {
  const { tText } = useTranslation();
  const value = activeKey ?? defaultActiveKey ?? items?.[0]?.key;
  const renderTabChildren = (item: NonNullable<typeof items>[number], active: boolean) => (
    typeof item.children === 'function' ? item.children(active) : item.children
  );

  return (
    <RadixTabs.Root value={value} onValueChange={onChange} className={cx('ui-tabs', className)} style={style}>
      <RadixTabs.List className="ui-tabs-list" onContextMenu={onTabListContextMenu}>
        {items?.map((item) => {
          const closeLabel = item.closeLabel ?? tText('common.closeTab');

          return (
            <RadixTabs.Trigger key={item.key} value={item.key} asChild>
              <div
                className="ui-tabs-trigger"
                onContextMenu={(event) => {
                  event.stopPropagation();
                  onTabContextMenu?.(event, item.key);
                }}
              >
                {item.label}
                {item.closable ? (
                  <button
                    type="button"
                    className="ui-tabs-close"
                    aria-label={closeLabel}
                    title={closeLabel}
                    onPointerDown={(event) => { event.stopPropagation(); }}
                    onMouseDown={(event) => { event.stopPropagation(); }}
                    onKeyDown={(event) => { event.stopPropagation(); }}
                    onClick={(event) => { event.stopPropagation(); onEdit?.(item.key, 'remove'); }}
                  >
                    ×
                  </button>
                ) : null}
              </div>
            </RadixTabs.Trigger>
          );
        })}
        {tabBarExtraContent}
      </RadixTabs.List>
      {items?.map((item) => {
        const isActive = item.key === value;
        if (destroyInactiveTabPane) {
          return (
            <RadixTabs.Content key={item.key} value={item.key} className="ui-tabs-content">
              {isActive ? renderTabChildren(item, true) : null}
            </RadixTabs.Content>
          );
        }

        return (
          <RadixTabs.Content key={item.key} value={item.key} className="ui-tabs-content" forceMount>
            {renderTabChildren(item, isActive)}
          </RadixTabs.Content>
        );
      })}
    </RadixTabs.Root>
  );
}

export function Switch({ checked, defaultChecked, onChange, onCheckedChange, disabled }: {
  checked?: boolean;
  defaultChecked?: boolean;
  onChange?: (checked: boolean) => void;
  onCheckedChange?: (checked: boolean) => void;
  disabled?: boolean;
  size?: Size;
}) {
  return (
    <RadixSwitch.Root
      checked={checked}
      defaultChecked={defaultChecked}
      disabled={disabled}
      onCheckedChange={(next) => { onCheckedChange?.(next); onChange?.(next); }}
      className="ui-switch"
    >
      <RadixSwitch.Thumb className="ui-switch-thumb" />
    </RadixSwitch.Root>
  );
}

export function Badge({ status = 'default', text }: BadgeProps) {
  return <span className="ui-badge"><span className={cx('ui-badge-dot', `ui-badge-${status}`)} />{text ? <span>{text}</span> : null}</span>;
}

export function Tag({ children, color, className, ...props }: { children?: ReactNode; color?: string } & HTMLAttributes<HTMLSpanElement>) {
  return <span {...props} className={cx('ui-tag', color && `ui-tag-${color}`, className)}>{children}</span>;
}

export function Card({ children, title, extra, size, className, styles, ...props }: {
  children?: ReactNode;
  title?: ReactNode;
  extra?: ReactNode;
  size?: Size;
  styles?: { body?: CSSProperties };
} & HTMLAttributes<HTMLDivElement>) {
  return (
    <section {...props} className={cx('ui-card', size === 'small' && 'ui-card-small', className)}>
      {(title || extra) && <header className="ui-card-header"><strong>{title}</strong><div>{extra}</div></header>}
      <div className="ui-card-body" style={styles?.body}>{children}</div>
    </section>
  );
}

export function Empty({ description, children }: { description?: ReactNode; image?: ReactNode; children?: ReactNode; styles?: { footer?: CSSProperties } }) {
  const { t } = useTranslation();
  return <div className="ui-empty"><div className="ui-empty-icon">∅</div><div>{description ?? t('common.noData')}</div>{children}</div>;
}
Empty.PRESENTED_IMAGE_SIMPLE = 'simple';

export function Spin({ size }: { size?: Size }) {
  return <span className={cx('ui-spinner', size === 'small' && 'ui-spinner-small')} />;
}

export function Statistic({ title, value, valueStyle, prefix }: { title?: ReactNode; value?: ReactNode; valueStyle?: CSSProperties; prefix?: ReactNode }) {
  return <div className="ui-statistic"><div className="ui-statistic-title">{title}</div><div className="ui-statistic-value" style={valueStyle}>{prefix}{value}</div></div>;
}

export function Result({ status, title, subTitle, extra }: { status?: string; title?: ReactNode; subTitle?: ReactNode; extra?: ReactNode }) {
  return <div className={cx('ui-result', status && `ui-result-${status}`)}><h3>{title}</h3>{subTitle ? <p>{subTitle}</p> : null}<div>{extra}</div></div>;
}

export function Row({ children, gutter = 0, style, className, align }: { children?: ReactNode; gutter?: number; style?: CSSProperties; className?: string; align?: string }) {
  return <div className={cx('ui-row', className)} style={{ gap: gutter, alignItems: align === 'middle' ? 'center' : align, ...style }}>{children}</div>;
}

export function Col({ children, span, style, className }: { children?: ReactNode; span?: number; style?: CSSProperties; className?: string }) {
  return <div className={cx('ui-col', className)} style={{ flex: span ? `0 0 ${span / 24 * 100}%` : undefined, maxWidth: span ? `${span / 24 * 100}%` : undefined, ...style }}>{children}</div>;
}

export const Layout = Object.assign(
  function LayoutRoot({ children, className, ...props }: HTMLAttributes<HTMLDivElement>) {
    return <div {...props} className={className}>{children}</div>;
  },
  {
    Content: function Content({ children, className, ...props }: HTMLAttributes<HTMLDivElement>) {
      return <div {...props} className={className}>{children}</div>;
    },
  },
);

export function Table<T>({
  rowKey = 'id',
  columns,
  dataSource,
  loading,
  rowSelection,
  pagination,
  scroll,
  locale,
  expandable,
  className,
}: {
  rowKey?: keyof T | string | ((record: T) => string);
  columns: ColumnType<T>[];
  dataSource?: T[];
  loading?: boolean;
  rowSelection?: { selectedRowKeys?: Key[]; onChange?: (keys: Key[]) => void };
  pagination?: false | { pageSize?: number; showSizeChanger?: boolean; showTotal?: (total: number) => ReactNode };
  scroll?: { y?: number | string };
  locale?: { emptyText?: ReactNode };
  expandable?: { defaultExpandAllRows?: boolean; expandedRowRender?: (record: T) => ReactNode };
  size?: Size;
  className?: string;
}) {
  const { t, tText } = useTranslation();
  const [sort, setSort] = useState<{ key: string; desc: boolean } | null>(null);
  const [page, setPage] = useState(1);
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const rows = dataSource ?? [];
  const sortedRows = useMemo(() => {
    if (!sort) return rows;
    const column = columns.find((item, index) => (item.key ?? String(item.dataIndex ?? index)) === sort.key);
    if (!column?.sorter) return rows;
    return [...rows].sort((a, b) => sort.desc ? column.sorter!(b, a) : column.sorter!(a, b));
  }, [columns, rows, sort]);
  const pageSize = pagination ? pagination.pageSize ?? 20 : sortedRows.length || 1;
  const pagedRows = useMemo(() => (
    pagination === false ? sortedRows : sortedRows.slice((page - 1) * pageSize, page * pageSize)
  ), [page, pageSize, pagination, sortedRows]);
  const selectedKeys = useMemo(() => new Set((rowSelection?.selectedRowKeys ?? []).map(String)), [rowSelection?.selectedRowKeys]);
  const allVisibleKeys = useMemo(
    () => pagedRows.map((record: T, index: number) => getRowKey(rowKey, record, index)),
    [pagedRows, rowKey],
  );
  const allSelected = allVisibleKeys.length > 0 && allVisibleKeys.every((key: string) => selectedKeys.has(key));

  useEffect(() => {
    const totalPages = Math.max(1, Math.ceil(sortedRows.length / pageSize));
    if (page > totalPages) {
      setPage(totalPages);
    }
  }, [page, pageSize, sortedRows.length]);

  return (
    <div className={cx('ui-table-wrapper', className)}>
      <div className="ui-table-scroll" style={{ maxHeight: scroll?.y, overflow: scroll?.y ? 'auto' : undefined }}>
        <table className="ui-table">
          <thead>
            <tr>
              {rowSelection && (
                <th style={{ width: 36 }}>
                  <input
                    type="checkbox"
                    checked={allSelected}
                    onChange={(event) => {
                      const next = new Set(selectedKeys);
                      allVisibleKeys.forEach((key) => event.target.checked ? next.add(key) : next.delete(key));
                      rowSelection.onChange?.(Array.from(next));
                    }}
                  />
                </th>
              )}
              {expandable && <th style={{ width: 36 }} />}
              {columns.map((column, index) => {
                const key = column.key ?? String(column.dataIndex ?? index);
                return (
                  <th key={key} style={{ width: column.width }}>
                    <button
                      type="button"
                      className={cx('ui-table-sort', column.sorter && 'ui-table-sortable')}
                      onClick={() => column.sorter && setSort((prev) => prev?.key === key ? { key, desc: !prev.desc } : { key, desc: false })}
                    >
                      {column.title}{sort?.key === key ? (sort.desc ? ' ↓' : ' ↑') : null}
                    </button>
                  </th>
                );
              })}
            </tr>
          </thead>
          <tbody>
            {loading ? (
              <tr><td colSpan={columns.length + (rowSelection ? 1 : 0) + (expandable ? 1 : 0)}><div className="ui-table-state"><Spin /> {t('common.loading')}</div></td></tr>
            ) : pagedRows.length === 0 ? (
              <tr><td colSpan={columns.length + (rowSelection ? 1 : 0) + (expandable ? 1 : 0)}>{locale?.emptyText ?? <Empty />}</td></tr>
            ) : pagedRows.map((record: T, rowIndex: number) => {
              const key = getRowKey(rowKey, record, rowIndex);
              const isExpanded = expandable?.defaultExpandAllRows || expanded.has(key);
              return (
                <FragmentRow key={key}>
                  <tr>
                    {rowSelection && (
                      <td>
                        <input
                          type="checkbox"
                          checked={selectedKeys.has(key)}
                          onChange={(event) => {
                            const next = new Set(selectedKeys);
                            event.target.checked ? next.add(key) : next.delete(key);
                            rowSelection.onChange?.(Array.from(next));
                          }}
                        />
                      </td>
                    )}
                    {expandable && (
                      <td><Button size="small" type="text" onClick={() => setExpanded((prev) => {
                        const next = new Set(prev);
                        next.has(key) ? next.delete(key) : next.add(key);
                        return next;
                      })}>{isExpanded ? '−' : '+'}</Button></td>
                    )}
                    {columns.map((column, columnIndex) => {
                      const value = readValue(record, column.dataIndex);
                      const rendered = column.render ? column.render(value as never, record, rowIndex) : (value as ReactNode);
                      return <td key={column.key ?? String(column.dataIndex ?? columnIndex)} className={column.ellipsis ? 'ui-table-ellipsis' : undefined}>{rendered}</td>;
                    })}
                  </tr>
                  {expandable && isExpanded && <tr><td colSpan={columns.length + (rowSelection ? 1 : 0) + 1}>{expandable.expandedRowRender?.(record)}</td></tr>}
                </FragmentRow>
              );
            })}
          </tbody>
        </table>
      </div>
      {pagination !== false && sortedRows.length > pageSize ? (
        <div className="ui-pagination">
          <span>{pagination ? pagination.showTotal?.(sortedRows.length) ?? tText('common.totalItems', { count: sortedRows.length }) : tText('common.totalItems', { count: sortedRows.length })}</span>
          <Button size="small" disabled={page <= 1} onClick={() => setPage((current) => Math.max(1, current - 1))}>{t('common.prevPage')}</Button>
          <span>{page} / {Math.ceil(sortedRows.length / pageSize)}</span>
          <Button size="small" disabled={page >= Math.ceil(sortedRows.length / pageSize)} onClick={() => setPage((current) => Math.min(Math.ceil(sortedRows.length / pageSize), current + 1))}>{t('common.nextPage')}</Button>
        </div>
      ) : null}
    </div>
  );
}

function FragmentRow({ children }: { children: ReactNode }) {
  return <>{children}</>;
}

export function Tree({
  treeData,
  checkedKeys,
  expandedKeys,
  onCheck,
  onExpand,
  className,
}: {
  treeData?: DataNode[];
  checkedKeys?: Key[];
  expandedKeys?: Key[];
  onCheck?: (keys: Key[] | { checked: Key[]; halfChecked: Key[] }) => void;
  onExpand?: (keys: Key[]) => void;
  checkable?: boolean;
  selectable?: boolean;
  blockNode?: boolean;
  className?: string;
}) {
  const checked = new Set((checkedKeys ?? []).map(String));
  const expanded = new Set((expandedKeys ?? []).map(String));

  const nodeMap = useMemo(() => {
    const map = new Map<string, DataNode>();
    const walk = (nodes?: DataNode[]) => nodes?.forEach((n) => { map.set(String(n.key), n); walk(n.children); });
    walk(treeData);
    return map;
  }, [treeData]);

  const collectDescendantKeys = (key: string): string[] => {
    const keys: string[] = [];
    const walk = (nodes?: DataNode[]) => nodes?.forEach((n) => { keys.push(String(n.key)); walk(n.children); });
    walk(nodeMap.get(key)?.children);
    return keys;
  };

  const isSubtreeFullyChecked = (key: string): boolean => {
    const node = nodeMap.get(key);
    if (!node?.children?.length) return checked.has(key);
    return node.children.every((c) => isSubtreeFullyChecked(String(c.key)));
  };

  const getCheckState = (key: string): { checked: boolean; indeterminate: boolean } => {
    const node = nodeMap.get(key);
    if (!node?.children?.length) return { checked: checked.has(key), indeterminate: false };
    const childStates = node.children.map((c) => getCheckState(String(c.key)));
    const allChecked = childStates.every((s) => s.checked);
    const someChecked = childStates.some((s) => s.checked || s.indeterminate);
    return { checked: allChecked, indeterminate: !allChecked && someChecked };
  };

  const toggleCheck = (key: string) => {
    const allCurrentChecked = isSubtreeFullyChecked(key);
    const next = new Set(checked);
    if (allCurrentChecked) {
      next.delete(key);
      collectDescendantKeys(key).forEach((k) => next.delete(k));
    } else {
      next.add(key);
      collectDescendantKeys(key).forEach((k) => next.add(k));
    }
    onCheck?.(Array.from(next) as Key[]);
  };
  const toggleExpand = (key: string) => {
    const next = new Set(expanded);
    next.has(key) ? next.delete(key) : next.add(key);
    onExpand?.(Array.from(next) as Key[]);
  };
  const renderNodes = (nodes?: DataNode[], depth = 0): ReactNode => nodes?.map((node) => {
    const key = String(node.key);
    const hasChildren = Boolean(node.children?.length);
    const isExpanded = !hasChildren || expanded.has(key);
    const checkState = getCheckState(key);
    return (
      <div key={key} className="ui-tree-node" style={{ paddingLeft: depth * 12 }}>
        <div className="ui-tree-row">
          {hasChildren ? <button type="button" className="ui-tree-toggle" onClick={() => toggleExpand(key)}>{isExpanded ? '▾' : '▸'}</button> : <span className="ui-tree-toggle" />}
          <input
            type="checkbox"
            checked={checkState.checked}
            ref={(el) => { if (el) el.indeterminate = checkState.indeterminate; }}
            onChange={() => toggleCheck(key)}
            disabled={node.disabled}
          />
          <span className="ui-tree-title">{node.title}</span>
        </div>
        {isExpanded ? renderNodes(node.children, depth + 1) : null}
      </div>
    );
  });
  return <div className={cx('ui-tree', className)}>{renderNodes(treeData)}</div>;
}

export interface DataNode {
  key: Key;
  title?: ReactNode;
  children?: DataNode[];
  selectable?: boolean;
  disabled?: boolean;
  isLeaf?: boolean;
}

// ---------------------------------------------------------------------------
// TreeSelect — project wrapper around TreeselectJS
// ---------------------------------------------------------------------------

export type TreeSelectOption = {
  key: string;
  title: ReactNode;
  children?: TreeSelectOption[];
  disabled?: boolean;
};

interface TreeSelectProps {
  options: TreeSelectOption[];
  value?: string | null;
  onChange?: (value: string | null) => void;
  placeholder?: string;
  disabled?: boolean;
  style?: CSSProperties;
  className?: string;
  allowClear?: boolean;
}

export function TreeSelect({
  options,
  value,
  onChange,
  placeholder,
  disabled,
  style,
  className,
  allowClear = true,
}: TreeSelectProps) {
  const { tText } = useTranslation();
  const containerRef = useRef<HTMLDivElement>(null);
  const treeselectRef = useRef<Treeselect | null>(null);
  const onChangeRef = useRef(onChange);
  const treeData = useMemo(() => options.map(toTreeselectOption), [options]);
  const selectedValue = normalizeTreeSelectInputValue(value);

  useEffect(() => {
    onChangeRef.current = onChange;
  }, [onChange]);

  useEffect(() => {
    const container = containerRef.current;
    if (!container) return undefined;

    const treeselect = new Treeselect({
      parentHtmlContainer: container,
      value: selectedValue,
      options: treeData,
      openLevel: 99,
      clearable: allowClear,
      searchable: false,
      placeholder: placeholder ?? tText('common.selectPlaceholder'),
      disabled,
      isSingleSelect: true,
      showTags: false,
      direction: 'bottom',
      listClassName: 'ui-tree-select-dropdown',
      inputCallback: (nextValue) => {
        onChangeRef.current?.(normalizeTreeselectValue(nextValue));
      },
    });

    treeselect.mount();
    treeselectRef.current = treeselect;

    return () => {
      treeselect.destroy();
      if (treeselectRef.current === treeselect) {
        treeselectRef.current = null;
      }
    };
  }, [allowClear, disabled, placeholder, tText, treeData]);

  useEffect(() => {
    treeselectRef.current?.updateValue(selectedValue);
  }, [selectedValue]);

  return (
    <div
      ref={containerRef}
      className={cx('ui-tree-select', className)}
      style={{ width: '100%', ...style }}
    />
  );
}

function normalizeTreeSelectInputValue(value: string | null | undefined): string | null {
  return value == null || value === '' ? null : value;
}

function normalizeTreeselectValue(value: ValueType): string | null {
  if (Array.isArray(value)) {
    return value.length > 0 ? String(value[0]) : null;
  }

  if (value == null || value === '') return null;
  return String(value);
}

function toTreeselectOption(opt: TreeSelectOption): OptionType {
  return {
    value: opt.key,
    name: getTreeSelectOptionName(opt.title),
    disabled: opt.disabled,
    isGroupSelectable: !opt.disabled,
    children: opt.children?.map(toTreeselectOption) ?? [],
  };
}

function getTreeSelectOptionName(title: ReactNode): string {
  if (typeof title === 'string' || typeof title === 'number') return String(title);
  if (Array.isArray(title)) return title.map(getTreeSelectOptionName).join('');
  if (isValidElement<{ children?: ReactNode }>(title)) return getTreeSelectOptionName(title.props.children);
  return title == null || typeof title === 'boolean' ? '' : String(title);
}

export const message = {
  success: (content: ReactNode) => toast(String(content), 'success'),
  error: (content: ReactNode) => toast(String(content), 'error'),
  warning: (content: ReactNode) => toast(String(content), 'warning'),
  info: (content: ReactNode) => toast(String(content), 'info'),
};

function toast(content: string, status: BadgeStatus | 'info') {
  window.dispatchEvent(new CustomEvent('opsbatch-toast', { detail: { content, status } }));
}

export function ToastProvider({ children }: { children: ReactNode }) {
  const [items, setItems] = useState<Array<{ id: number; content: string; status: string }>>([]);
  useEffect(() => {
    const handler = (event: Event) => {
      const detail = (event as CustomEvent<{ content: string; status: string }>).detail;
      const id = Date.now() + Math.random();
      setItems((prev) => [...prev, { id, ...detail }]);
      window.setTimeout(() => setItems((prev) => prev.filter((item) => item.id !== id)), 2800);
    };
    window.addEventListener('opsbatch-toast', handler);
    return () => window.removeEventListener('opsbatch-toast', handler);
  }, []);
  return (
    <RadixToast.Provider swipeDirection="right">
      {children}
      {items.map((item) => (
        <RadixToast.Root key={item.id} className={cx('ui-toast', `ui-toast-${item.status}`)} open>
          <RadixToast.Title>{item.content}</RadixToast.Title>
        </RadixToast.Root>
      ))}
      <RadixToast.Viewport className="ui-toast-viewport" />
    </RadixToast.Provider>
  );
}

export const Radio = {
  Group({ value, onChange, children }: { value?: string; onChange?: (event: { target: { value: string } }) => void; children?: ReactNode }) {
    return (
      <div className="ui-radio-group" data-value={value} onClick={(event) => {
        const target = event.target as HTMLElement;
        const button = target.closest('button[value]') as HTMLButtonElement | null;
        if (button) onChange?.({ target: { value: button.value } });
      }}>
        {children}
      </div>
    );
  },
  Button({ value, children }: { value: string; children?: ReactNode }) {
    return <button type="button" className="ui-radio-button" value={value}>{children}</button>;
  },
};

export function Divider() {
  return <hr className="ui-divider" />;
}
