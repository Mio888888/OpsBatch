import { useState, useMemo } from 'react';
import { Input } from '../../../components/ui';
import { useTranslation } from '../../../i18n';
import { NODE_GROUPS } from './nodeTypes';

interface Props {
  onAddNode: (type: string) => void;
}

export default function NodePalette({ onAddNode }: Props) {
  const { tText } = useTranslation();
  const [search, setSearch] = useState('');
  const q = search.toLowerCase();

  const groups = useMemo(() => {
    const localizedGroups = NODE_GROUPS.map((group) => ({
      ...group,
      label: tText(group.labelKey),
      types: group.types.map((type) => ({
        ...type,
        label: tText(type.labelKey),
      })),
    }));

    if (!q) return localizedGroups;
    return localizedGroups.map((group) => ({
      ...group,
      types: group.types.filter((type) => (
        type.label.toLowerCase().includes(q)
        || type.fallbackLabel.toLowerCase().includes(q)
        || type.value.toLowerCase().includes(q)
      )),
    })).filter((group) => group.types.length > 0);
  }, [q, tText]);

  return (
    <div className="wf-palette">
      <div className="wf-palette-header">
        <span style={{ fontSize: 13, fontWeight: 600 }}>{tText('nodePalette.title')}</span>
      </div>
      <div className="wf-palette-search">
        <Input placeholder={tText('nodePalette.search')} size="small" value={search} onChange={(e) => setSearch(e.target.value)} style={{ fontSize: 12 }} allowClear />
      </div>
      <div className="wf-palette-list">
        {groups.map((group) => (
          <div key={group.labelKey} className="wf-palette-group">
            <div className="wf-palette-group-label">{group.label}</div>
            {group.types.map((type) => (
              <div
                key={type.value}
                className="wf-palette-item"
                onClick={() => onAddNode(type.value)}
              >
                <span className="wf-palette-dot" style={{ background: type.color }} />
                <span style={{ fontSize: 12 }}>{type.label}</span>
              </div>
            ))}
          </div>
        ))}
      </div>
    </div>
  );
}
