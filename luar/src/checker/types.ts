import type { MethodMember, FieldMember, AccessMod } from "../parser/ast.js";

export type CheckError = {
  message: string;
  line: number;
  col: number;
};

export type MethodInfo = {
  method: MethodMember;
  access: AccessMod;
  className: string;
};

export type FieldInfo = {
  field: FieldMember;
  access: AccessMod;
  className: string;
};

export type ClassInfo = {
  name: string;
  isAbstract: boolean;
  parentName: string | null;
  methods: Map<string, MethodInfo>;
  fields: Map<string, FieldInfo>;
  line: number;
};
