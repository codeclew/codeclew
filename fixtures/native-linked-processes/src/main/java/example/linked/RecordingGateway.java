package example.linked;

import java.util.ArrayList;
import java.util.List;

/** Records local attempts; zero is this fixture's acknowledgement convention. */
public final class RecordingGateway implements Gateway {
    private final int response;
    private final boolean fail;
    private final List<String> requests = new ArrayList<>();

    public RecordingGateway(int response, boolean fail) {
        this.response = response;
        this.fail = fail;
    }

    @Override
    public int deliver(String request) {
        requests.add(request);
        if (fail) {
            throw new IllegalStateException("fixture-local gateway failure");
        }
        return response;
    }

    public List<String> requests() {
        return List.copyOf(requests);
    }
}
