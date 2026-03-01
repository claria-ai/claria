import StepIndicator from "../components/StepIndicator";
import type { Page } from "../App";

export default function AccessKeyGuide({
  navigate,
}: {
  navigate: (page: Page) => void;
}) {
  return (
    <div className="max-w-2xl mx-auto p-8">
      <StepIndicator current={3} />

      <h2 className="text-2xl font-bold mb-6">
        Step 3: Create a Root Access Key
      </h2>

      <div className="space-y-4 text-gray-700">
        <p>
          Claria needs a one-time access key from the root account to set up
          your infrastructure. It will create a dedicated least-privilege user
          and then <strong>delete the root access key</strong> automatically.
        </p>

        <ol className="list-decimal list-inside space-y-3 pl-2">
          <li>
            Sign in to the{" "}
            <span className="font-mono text-sm bg-gray-100 px-1 py-0.5 rounded">
              AWS Management Console
            </span>{" "}
            as the <strong>root user</strong>
          </li>
          <li>
            Click your account name in the top-right corner and select{" "}
            <strong>Security credentials</strong>
          </li>
          <li>
            Scroll to <strong>Access keys</strong> and click{" "}
            <strong>"Create access key"</strong>
          </li>
          <li>
            Check the acknowledgment box and click{" "}
            <strong>"Create access key"</strong>
          </li>
        </ol>

        <div className="bg-amber-50 border border-amber-200 rounded-lg p-4 mt-6">
          <p className="text-amber-800 text-sm font-medium">
            Copy both the Access Key ID and Secret Access Key now — the secret
            is only shown once.
          </p>
        </div>

        <div className="bg-blue-50 border border-blue-200 rounded-lg p-4">
          <p className="text-blue-800 text-sm">
            You will paste these into Claria on the next screen. Claria will use
            them to create a scoped IAM user with minimal permissions and then
            delete this root access key from your account.
          </p>
        </div>
      </div>

      <div className="flex justify-between mt-8">
        <button
          onClick={() => navigate("guide-mfa")}
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
