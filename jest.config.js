/** @type {import('jest').Config} */
export default {
  rootDir: ".",
  testEnvironment: "node",
  transform: {
    "^.+\\.tsx?$": [
      "ts-jest",
      {
        useESM: true,
      },
    ],
  },
  extensionsToTreatAsEsm: [".ts"],
  moduleNameMapper: {
    "^(\\.{1,2}/.*)\\.js$": "$1",
    "^@opensymphony/gateway-schema$": "<rootDir>/packages/gateway-schema/src/index.ts",
    "^@opensymphony/gateway-schema/(.+)$": "<rootDir>/packages/gateway-schema/src/$1",
  },
  testMatch: ["**/__tests__/**/*.test.ts"],
  transformIgnorePatterns: ["/node_modules/"],
};