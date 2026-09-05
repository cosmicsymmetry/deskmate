import type { ValidationIssue } from "../lib/types";

interface FieldIssuesProps {
  issues: ValidationIssue[];
  className?: string;
}

export function FieldIssues({ issues, className }: FieldIssuesProps) {
  if (issues.length === 0) {
    return null;
  }
  return (
    <ul className={`field-errors${className ? ` ${className}` : ""}`} role="alert">
      {issues.map((issue) => (
        <li key={`${issue.path}:${issue.code}`}>{issue.message}</li>
      ))}
    </ul>
  );
}
