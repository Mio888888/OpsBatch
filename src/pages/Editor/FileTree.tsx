import { memo, useState, useCallback } from 'react';
import { useEditorStore, type TreeNode } from '../../stores/editor';
import type { FC } from 'react';

const FileTreeFileNode: FC<{
  name: string;
  path: string;
  depth: number;
  onSelect: (path: string) => void;
}> = memo(function FileTreeFileNode({ name, path, depth, onSelect }) {
  const isActive = useEditorStore((state) => state.currentFilePath === path);

  const handleSelect = useCallback(() => {
    onSelect(path);
  }, [path, onSelect]);

  return (
    <div
      className={`editor-tree-item ${isActive ? 'editor-tree-item-active' : ''}`}
      style={{ paddingLeft: depth * 16 + 8 }}
      onClick={handleSelect}
    >
      <span className="editor-tree-icon">📄</span>
      <span className="editor-tree-name">{name}</span>
    </div>
  );
});

const FileTreeNode: FC<{
  node: TreeNode;
  depth: number;
  onSelect: (path: string) => void;
}> = memo(function FileTreeNode({ node, depth, onSelect }) {
  const [expanded, setExpanded] = useState(depth < 1);

  const handleToggle = useCallback(() => {
    setExpanded((value) => !value);
  }, []);

  if (!node.is_dir) {
    return (
      <FileTreeFileNode
        name={node.name}
        path={node.path}
        depth={depth}
        onSelect={onSelect}
      />
    );
  }

  return (
    <div>
      <div
        className="editor-tree-item editor-tree-dir"
        style={{ paddingLeft: depth * 16 + 8 }}
        onClick={handleToggle}
      >
        <span className="editor-tree-toggle">{expanded ? '▾' : '▸'}</span>
        <span className="editor-tree-icon">{expanded ? '📂' : '📁'}</span>
        <span className="editor-tree-name">{node.name}</span>
      </div>
      {expanded && node.children.map((child) => (
        <FileTreeNode
          key={child.path}
          node={child}
          depth={depth + 1}
          onSelect={onSelect}
        />
      ))}
    </div>
  );
});

function FileTreeSkeleton() {
  const rows = [0, 1, 2, 1, 2, 2, 1, 2, 3, 1];

  return (
    <div className="editor-tree-skeleton" role="status" aria-label="正在加载目录树…">
      {rows.map((depth, index) => (
        <div
          key={`${depth}-${index}`}
          className="editor-tree-skeleton-row"
          style={{ paddingLeft: depth * 14 + 10 }}
        >
          <span className="editor-skeleton-dot" />
          <span className={`editor-tree-skeleton-name editor-tree-skeleton-name-${index % 3}`} />
        </div>
      ))}
    </div>
  );
}

interface FileTreeProps {
  tree: TreeNode | null;
  loading: boolean;
}

const FileTree: FC<FileTreeProps> = memo(function FileTree({ tree, loading }) {
  const openFileInDir = useEditorStore((state) => state.openFileInDir);

  const handleSelect = useCallback((path: string) => {
    void openFileInDir(path);
  }, [openFileInDir]);

  if (loading) {
    return <FileTreeSkeleton />;
  }

  if (!tree) {
    return <div className="editor-tree-empty">空目录</div>;
  }

  return (
    <div className="editor-tree">
      {tree.children.map((child) => (
        <FileTreeNode
          key={child.path}
          node={child}
          depth={0}
          onSelect={handleSelect}
        />
      ))}
    </div>
  );
});

export default FileTree;
