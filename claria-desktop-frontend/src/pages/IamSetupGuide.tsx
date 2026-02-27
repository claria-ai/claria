import StepIndicator from "../components/StepIndicator";
import type { Page } from "../App";

export default function IamSetupGuide({
  navigate,
}: {
  navigate: (page: Page) => void;
}) {
  return (
    <div className="max-w-2xl mx-auto p-8">
      <StepIndicator current={2} />

      <h2 className="text-2xl font-bold mb-6">Step 2: Create IAM Credentials</h2>

      <div className="space-y-4 text-gray-700">
        <p>
          Claria needs AWS credentials to provision and manage resources. Create
          a dedicated IAM user with the right permissions:
        </p>

        <ol className="list-decimal list-inside space-y-3 pl-2">
          <li>
            Open the{" "}
            <span className="font-mono text-sm bg-gray-100 px-1 py-0.5 rounded">
              IAM console
            </span>{" "}
            in your AWS account
          </li>
          <li>
            Go to <strong>Users</strong> and click{" "}
            <strong>"Create user"</strong>
          </li>
          <li>
            Name it something like{" "}
            <span className="font-mono text-sm bg-gray-100 px-1 py-0.5 rounded">
              claria-admin
            </span>
          </li>
          <li>
            Attach the{" "}
            <span className="font-mono text-sm bg-gray-100 px-1 py-0.5 rounded">
              AdministratorAccess
            </span>{" "}
            managed policy. (Future versions of Claria will use a scoped-down
            policy with least privilege.)
          </li>
          <li>
            Go to the user's <strong>Security credentials</strong> tab and click{" "}
            <strong>"Create access key"</strong>
          </li>
          <li>
            Select <strong>"Application running outside AWS"</strong> as the use
            case
          </li>
        </ol>

        <div className="bg-amber-50 border border-amber-200 rounded-lg p-4 mt-6">
          <p className="text-amber-800 text-sm font-medium">
            Copy both the Access Key ID and Secret Access Key now — the secret
            is only shown once.
          </p>
        </div>
      </div>

      <div className="flex justify-between mt-8">
        <button
          onClick={() => navigate("guide-aws")}
          className="px-4 py-2 text-gray-600 hover:text-gray-800"
        >
          Back
        </button>
        <button
          onClick={() => navigate("credentials")}
          className="px-6 py-2 bg-blue-500 text-white rounded-lg hover:bg-blue-600 transition-colors"
        >
          Next
        </button>
      </div>
    </div>
  );
}
